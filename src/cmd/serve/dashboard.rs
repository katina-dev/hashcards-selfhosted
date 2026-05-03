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

use std::collections::HashMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use chrono::Datelike;
use chrono::Duration;
use maud::Markup;
use maud::html;

use crate::cmd::serve::state::AppState;
use crate::cmd::serve::template::page_template;
use crate::error::Fallible;
use crate::types::date::Date;
use crate::types::timestamp::Timestamp;

const WEEKS: i64 = 53;
const CELL: i32 = 12;
const GAP: i32 = 2;

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
    let heatmap = heatmap_block(&state, today)?;

    let body = html! {
        div.dashboard {
            (status_block(due_count))
            (heatmap)
            (footer_block(last_reviewed))
        }
    };
    Ok(page_template(body))
}

fn heatmap_block(state: &AppState, today: Date) -> Fallible<Markup> {
    let start = Date::new(today.into_inner() + Duration::days(-(WEEKS * 7 - 1)));
    let counts: Vec<(Date, u32)> = {
        let db = state.db.lock().unwrap();
        db.count_reviews_in_date_range(start, today)?
    };
    let map: HashMap<Date, u32> = counts.into_iter().collect();

    let mut svg = String::new();
    let width = WEEKS as i32 * (CELL + GAP);
    let height = 7 * (CELL + GAP);
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\" class=\"heatmap-svg\">",
        width, height, width, height
    ));
    let start_naive = start.into_inner();
    let weekday_offset = start_naive.weekday().num_days_from_sunday() as i64;
    let grid_start = start_naive - Duration::days(weekday_offset);

    for w in 0..WEEKS {
        for d in 0..7 {
            let date = grid_start + Duration::days(w * 7 + d);
            if date < start_naive || date > today.into_inner() {
                continue;
            }
            let count = map.get(&Date::new(date)).copied().unwrap_or(0);
            let class = bucket_class(count);
            let x = w as i32 * (CELL + GAP);
            let y = d as i32 * (CELL + GAP);
            svg.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" class=\"hm-cell hm-{}\"><title>{}: {}</title></rect>",
                x, y, CELL, CELL, class, Date::new(date), count
            ));
        }
    }
    svg.push_str("</svg>");
    Ok(html! {
        div.heatmap {
            (maud::PreEscaped(svg))
        }
    })
}

fn bucket_class(count: u32) -> &'static str {
    match count {
        0 => "0",
        1..=5 => "1",
        6..=15 => "2",
        16..=30 => "3",
        _ => "4",
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
