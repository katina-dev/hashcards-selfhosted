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

use crate::cmd::serve::state::Review;
use crate::cmd::serve::state::ServerState;
use crate::error::Fallible;
use crate::fsrs::Grade;
use crate::types::card::Card;
use crate::types::card_hash::CardHash;
use crate::types::performance::Performance;
use crate::types::performance::ReviewedPerformance;
use crate::types::performance::update_performance;
use crate::types::timestamp::Timestamp;

#[derive(Debug, Deserialize)]
enum Action {
    Reveal,
    Undo,
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
            Action::Reveal | Action::Undo => panic!("Action does not correspond to a grade"),
        }
    }
}

#[derive(Deserialize)]
pub struct FormData {
    action: Action,
}

pub async fn post_handler(
    State(state): State<ServerState>,
    Form(form): Form<FormData>,
) -> Redirect {
    if let Err(e) = action_handler(state, form.action).await {
        log::error!("error: {e}");
    }
    Redirect::to("/")
}

async fn action_handler(state: ServerState, action: Action) -> Fallible<()> {
    let mut mutable = state.mutable.lock().unwrap();
    match action {
        Action::Reveal => {
            mutable.reveal = true;
        }
        Action::Undo => {
            // Per-rating undo: best-effort, requires re-reading the previous
            // performance from DB and rolling back the last review row. The
            // current undo behavior of the drill UI relied on the cache; we
            // simplify by making Undo a no-op in serve mode (the request that
            // last rated a card has already been committed to disk). The Undo
            // button is hidden in get.rs.
        }
        Action::Forgot | Action::Hard | Action::Good | Action::Easy => {
            if !mutable.reveal {
                return Ok(());
            }
            if mutable.cards.is_empty() {
                return Ok(());
            }
            let reviewed_at: Timestamp = Timestamp::now();
            let card: Card = mutable.cards.remove(0);
            let hash: CardHash = card.hash();
            let grade: Grade = action.grade();

            // Read current performance from DB (no cache).
            let prior: Performance = mutable.db.get_card_performance(hash)?;
            let new_perf: ReviewedPerformance = update_performance(prior, grade, reviewed_at);
            let new_perf_enum = Performance::Reviewed(new_perf);

            let review = Review {
                card: card.clone(),
                reviewed_at,
                grade,
                stability: new_perf.stability,
                difficulty: new_perf.difficulty,
                interval_raw: new_perf.interval_raw,
                interval_days: new_perf.interval_days,
                due_date: new_perf.due_date,
            };

            // Persist atomically: review + performance update in one transaction.
            let record = review.clone().into_record();
            mutable.db.record_rating(state.session_id, &record, new_perf_enum)?;

            if review.should_repeat() {
                mutable.cards.push(card);
            }
            mutable.reviews.push(review);
            mutable.reveal = false;
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
