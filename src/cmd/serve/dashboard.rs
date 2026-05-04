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
const LABEL_LEFT: i32 = 28;
const LABEL_TOP: i32 = 16;
const LEGEND_HEIGHT: i32 = 24;
const RETENTION_WINDOW_DAYS: i64 = 30;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

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
    let heatmap = heatmap_block(&state, today)?;

    let body = html! {
        div.dashboard {
            (status_block(due_count))
            (stats_block(streak, total_reviews, retention))
            (heatmap)
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

fn heatmap_block(state: &AppState, today: Date) -> Fallible<Markup> {
    let start = Date::new(today.into_inner() + Duration::days(-(WEEKS * 7 - 1)));
    let counts: Vec<(Date, u32)> = {
        let db = state.db.lock().unwrap();
        db.count_reviews_in_date_range(start, today)?
    };
    let map: HashMap<Date, u32> = counts.into_iter().collect();

    let cells_width = WEEKS as i32 * (CELL + GAP);
    let cells_height = 7 * (CELL + GAP);
    let total_width = LABEL_LEFT + cells_width;
    let total_height = LABEL_TOP + cells_height + LEGEND_HEIGHT;

    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\" class=\"heatmap-svg\">",
        total_width, total_height, total_width, total_height
    ));

    // Weekday labels on the left (Mon, Wed, Fri).
    for &(row, label) in &[(1, "Mon"), (3, "Wed"), (5, "Fri")] {
        let y = LABEL_TOP + row * (CELL + GAP) + (CELL * 3 / 4);
        svg.push_str(&format!(
            "<text x=\"0\" y=\"{}\" class=\"hm-axis\">{}</text>",
            y, label
        ));
    }

    let start_naive = start.into_inner();
    let weekday_offset = start_naive.weekday().num_days_from_sunday() as i64;
    let grid_start = start_naive - Duration::days(weekday_offset);

    // Cells, plus collect month-transition columns for the top labels.
    let mut last_month: Option<u32> = None;
    let mut month_marks: Vec<(i32, &'static str)> = Vec::new();
    for w in 0..WEEKS {
        // Find the first in-range day of this column, then maybe emit a label.
        let mut first_in_range: Option<chrono::NaiveDate> = None;
        for d in 0..7 {
            let date = grid_start + Duration::days(w * 7 + d);
            if date < start_naive || date > today.into_inner() {
                continue;
            }
            first_in_range = Some(date);
            break;
        }
        if let Some(date) = first_in_range {
            let m = date.month();
            // Skip the very first column when it's a partial month (e.g.
            // a 2-day Apr fragment before May): wait until the column where
            // the month actually changes.
            let emit = match last_month {
                None => date.day() == 1,
                Some(prev) => prev != m,
            };
            if emit {
                let x = LABEL_LEFT + w as i32 * (CELL + GAP);
                month_marks.push((x, MONTHS[(m - 1) as usize]));
            }
            last_month = Some(m);
        }

        for d in 0..7 {
            let date = grid_start + Duration::days(w * 7 + d);
            if date < start_naive || date > today.into_inner() {
                continue;
            }
            let count = map.get(&Date::new(date)).copied().unwrap_or(0);
            let class = bucket_class(count);
            let x = LABEL_LEFT + w as i32 * (CELL + GAP);
            let y = LABEL_TOP + d as i32 * (CELL + GAP);
            svg.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" class=\"hm-cell hm-{}\"><title>{}: {}</title></rect>",
                x, y, CELL, CELL, class, Date::new(date), count
            ));
        }
    }

    // Month labels along the top. Skip a label if it would visually collide
    // with the previous one (less than ~3 weeks of horizontal space).
    let mut prev_x: Option<i32> = None;
    let min_gap = 3 * (CELL + GAP);
    for (x, name) in &month_marks {
        let too_close = match prev_x {
            Some(px) => x - px < min_gap,
            None => false,
        };
        if too_close {
            continue;
        }
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"12\" class=\"hm-axis\">{}</text>",
            x, name
        ));
        prev_x = Some(*x);
    }

    // Legend: Less [5 buckets] More.
    let legend_y = LABEL_TOP + cells_height + 8;
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" class=\"hm-axis\">Less</text>",
        LABEL_LEFT,
        legend_y + CELL - 2
    ));
    let legend_x_start = LABEL_LEFT + 32;
    for i in 0..5 {
        let x = legend_x_start + i * (CELL + GAP);
        svg.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" class=\"hm-cell hm-{}\"/>",
            x, legend_y, CELL, CELL, i
        ));
    }
    let more_x = legend_x_start + 5 * (CELL + GAP) + 4;
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" class=\"hm-axis\">More</text>",
        more_x,
        legend_y + CELL - 2
    ));

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
