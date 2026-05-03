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
    let last_reviewed = {
        let db = state.db.lock().unwrap();
        db.last_review_timestamp()?
    };

    let body = html! {
        div.dashboard {
            (status_block(due_count))
            // Heatmap section will be inserted by Task 16
            (footer_block(last_reviewed))
        }
    };
    Ok(page_template(body))
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
