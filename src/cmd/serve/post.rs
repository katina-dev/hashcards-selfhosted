// Copyright 2025–2026 Fernando Borretti
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use axum::Form;
use axum::extract::State;
use axum::response::Redirect;
use serde::Deserialize;

use crate::cmd::serve::state::AppState;
use crate::db::ReviewRecord;
use crate::error::Fallible;
use crate::fsrs::Grade;
use crate::types::performance::Performance;
use crate::types::performance::ReviewedPerformance;
use crate::types::performance::update_performance;
use crate::types::timestamp::Timestamp;

#[derive(Debug, Deserialize)]
enum Action {
    Undo,
    Reject,
    Forgot,
    Hard,
    Good,
    Easy,
}

impl Action {
    pub fn grade(&self) -> Grade {
        match self {
            Action::Forgot => Grade::Forgot,
            Action::Hard => Grade::Hard,
            Action::Good => Grade::Good,
            Action::Easy => Grade::Easy,
            Action::Undo | Action::Reject => panic!("Action does not correspond to a grade"),
        }
    }
}

#[derive(Deserialize)]
pub struct FormData {
    action: Action,
}

pub async fn post_handler(
    State(state): State<AppState>,
    Form(form): Form<FormData>,
) -> Redirect {
    if let Err(e) = action_handler(state, form.action).await {
        log::error!("error: {e}");
    }
    Redirect::to("/drill")
}

async fn action_handler(state: AppState, action: Action) -> Fallible<()> {
    match action {
        Action::Undo => {
            // Per-rating undo would require rolling back the last review row.
            // Serve mode treats Undo as a no-op (the rating is already on disk).
            // The Undo button is hidden in get.rs.
        }
        Action::Reject => {
            let current_hash = state.session_state.lock().unwrap().current_card;
            let hash = match current_hash {
                Some(h) => h,
                None => return Ok(()),
            };
            {
                let db = state.db.lock().unwrap();
                db.reject_card(hash, Timestamp::now())?;
            }
            let mut session = state.session_state.lock().unwrap();
            session.relapse_queue.retain(|h| h != &hash);
            session.current_card = None;
        }
        Action::Forgot | Action::Hard | Action::Good | Action::Easy => {
            let current_hash = state.session_state.lock().unwrap().current_card;
            let hash = match current_hash {
                Some(h) => h,
                None => return Ok(()),
            };

            // Confirm the card still exists in the index (it may have been
            // deleted between GET and POST by the file watcher).
            let exists = state
                .cards
                .read()
                .unwrap()
                .cards
                .iter()
                .any(|c| c.hash() == hash);
            if !exists {
                log::warn!(
                    "current_card hash not found in index (likely deleted between GET and POST)"
                );
                state.session_state.lock().unwrap().current_card = None;
                return Ok(());
            }

            let grade = action.grade();
            let reviewed_at = Timestamp::now();

            {
                let mut db = state.db.lock().unwrap();
                let prior: Performance = db.get_card_performance(hash)?;
                let new_perf: ReviewedPerformance = update_performance(prior, grade, reviewed_at);
                let new_perf_enum = Performance::Reviewed(new_perf);

                let record = ReviewRecord {
                    card_hash: hash,
                    reviewed_at,
                    grade,
                    stability: new_perf.stability,
                    difficulty: new_perf.difficulty,
                    interval_raw: new_perf.interval_raw,
                    interval_days: new_perf.interval_days,
                    due_date: new_perf.due_date,
                };
                db.record_rating(state.session_id, &record, new_perf_enum)?;
            }

            let mut session = state.session_state.lock().unwrap();
            if grade == Grade::Forgot || grade == Grade::Hard {
                // Re-show this session: MIN_INTERVAL=1 schedules it for tomorrow,
                // so the relapse queue surfaces it before other due cards today.
                if !session.relapse_queue.contains(&hash) {
                    session.relapse_queue.push(hash);
                }
            } else {
                session.relapse_queue.retain(|h| h != &hash);
            }
            session.current_card = None;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_grade() {
        assert_eq!(Action::Forgot.grade(), Grade::Forgot);
        assert_eq!(Action::Hard.grade(), Grade::Hard);
        assert_eq!(Action::Good.grade(), Grade::Good);
        assert_eq!(Action::Easy.grade(), Grade::Easy);
    }
}
