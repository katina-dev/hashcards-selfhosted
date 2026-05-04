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

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use maud::Markup;
use maud::html;

use crate::cmd::serve::state::AppState;
use crate::cmd::serve::template::page_template;
use crate::error::Fallible;
use crate::types::date::Date;
use crate::types::timestamp::Timestamp;

const RETENTION_WINDOW_DAYS: i64 = 30;

pub async fn dashboard_handler(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let html = match render_dashboard(state).await {
        Ok(m) => m,
        Err(e) => page_template(html! {
            div.error {
                h1 { "Error" }
                p { (e) }
            }
        }),
    };
    (StatusCode::OK, Html(html.into_string()))
}

async fn render_dashboard(state: AppState) -> Fallible<Markup> {
    let today = Date::today();
    let queue = state.compute_due_queue(today)?;
    let due_count = queue.len();
    let (last_reviewed, streak, total_reviews, retention) = {
        let db = state.db.lock().unwrap();
        (
            db.last_review_timestamp()?,
            db.current_streak(today)?,
            db.total_reviews()?,
            db.retention_rate(RETENTION_WINDOW_DAYS)?,
        )
    };

    let body = html! {
        div.dashboard {
            (status_block(due_count))
            (stats_block(streak, total_reviews, retention))
            (footer_block(last_reviewed))
        }
    };
    Ok(page_template(body))
}

fn stats_block(streak: u32, total_reviews: u64, retention: f32) -> Markup {
    let retention_str = if total_reviews == 0 {
        "—".to_string()
    } else {
        format!("{:.0}%", retention * 100.0)
    };
    html! {
        div.stats-row {
            div.stat {
                div.stat-value { (streak) }
                div.stat-label { "day streak" }
            }
            div.stat {
                div.stat-value { (total_reviews) }
                div.stat-label { "total reviews" }
            }
            div.stat {
                div.stat-value { (retention_str) }
                div.stat-label { "retention (30d)" }
            }
        }
    }
}

fn status_block(due_count: usize) -> Markup {
    if due_count == 0 {
        html! {
            div.status.caught-up {
                h1 { "You're caught up." }
            }
        }
    } else {
        html! {
            div.status {
                div.due-count { (due_count) " cards due" }
                a.btn-primary href="/drill" { "Drill" }
            }
        }
    }
}

fn footer_block(last: Option<Timestamp>) -> Markup {
    let last_str = match last {
        None => "Never drilled".to_string(),
        Some(ts) => format!("Last drilled: {}", ts.into_inner().format("%Y-%m-%d %H:%M")),
    };
    html! {
        div.footer {
            (last_str)
        }
    }
}
