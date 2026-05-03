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

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Display;
use std::fmt::Formatter;

use chrono::Datelike;
use chrono::Duration;
use clap::ValueEnum;
use maud::Markup;
use maud::html;
use serde::Serialize;

use crate::cmd::serve::template::page_template;
use crate::collection::Collection;
use crate::db::Database;
use crate::error::Fallible;
use crate::types::card::Card;
use crate::types::date::Date;
use crate::types::performance::Maturity;
use crate::types::performance::Performance;

#[derive(ValueEnum, Clone, Copy, PartialEq)]
pub enum StatsFormat {
    /// HTML output.
    Html,
    /// JSON output.
    Json,
}

impl Display for StatsFormat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            StatsFormat::Html => write!(f, "html"),
            StatsFormat::Json => write!(f, "json"),
        }
    }
}

pub fn print_stats(directory: Option<String>, format: StatsFormat) -> Fallible<()> {
    let coll = Collection::new(directory)?;
    match format {
        StatsFormat::Html => {
            let body = render_stats_page(&coll)?;
            println!("{}", page_template(body).into_string());
        }
        StatsFormat::Json => {
            let stats = build_json_stats(&coll)?;
            let stats_json = serde_json::to_string_pretty(&stats)?;
            println!("{}", stats_json);
        }
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    cards_in_deck_count: usize,
    cards_in_db_count: usize,
    tex_macro_count: usize,
    cards_reviewed_today_count: usize,
}

fn build_json_stats(coll: &Collection) -> Fallible<Stats> {
    let cards_in_db_count = coll.db.card_hashes()?.len();
    let today = Date::today();
    Ok(Stats {
        cards_in_deck_count: coll.cards.len(),
        cards_in_db_count,
        tex_macro_count: coll.macros.len(),
        cards_reviewed_today_count: coll.db.count_reviews_in_date(today)?,
    })
}

fn render_stats_page(coll: &Collection) -> Fallible<Markup> {
    let today = Date::today();
    let streak = coll.db.current_streak(today)?;
    let heatmap = heatmap_svg(&coll.db, today)?;
    let stats_grid = stats_grid_block(&coll.cards, &coll.db)?;
    let decks = per_deck_block(&coll.cards, &coll.db, today)?;
    Ok(html! {
        div.dashboard {
            h1 { "Detailed stats" }
            (streak_block(streak))
            (heatmap)
            (stats_grid)
            (decks)
        }
    })
}

fn streak_block(days: u32) -> Markup {
    if days == 0 {
        html! { div.streak { "0 day streak" } }
    } else {
        html! { div.streak { (days) " day streak" } }
    }
}

const WEEKS: i64 = 53;
const CELL: i32 = 12;
const GAP: i32 = 2;

fn heatmap_svg(db: &Database, today: Date) -> Fallible<Markup> {
    let start = Date::new(today.into_inner() + Duration::days(-(WEEKS * 7 - 1)));
    let counts = db.count_reviews_in_date_range(start, today)?;
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
    Ok(html! { div.heatmap { (maud::PreEscaped(svg)) } })
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

fn stats_grid_block(cards: &[Card], db: &Database) -> Fallible<Markup> {
    let total_cards = cards.len();
    let total_reviews = db.total_reviews()?;
    let today = Date::today();
    let reviewed_today = db.count_reviews_in_date(today)?;
    let retention = db.retention_rate(30)?;
    let mut new_n = 0usize;
    let mut learn_n = 0usize;
    let mut young_n = 0usize;
    let mut mature_n = 0usize;
    for c in cards {
        let perf: Performance = match db.get_card_performance_opt(c.hash())? {
            Some(p) => p,
            None => Performance::New,
        };
        match perf.maturity() {
            Maturity::New => new_n += 1,
            Maturity::Learning => learn_n += 1,
            Maturity::Young => young_n += 1,
            Maturity::Mature => mature_n += 1,
        }
    }
    Ok(html! {
        div.stats-grid {
            div.stat-tile { div.label { "Total cards" } div.value { (total_cards) } }
            div.stat-tile { div.label { "Total reviews" } div.value { (total_reviews) } }
            div.stat-tile { div.label { "Reviewed today" } div.value { (reviewed_today) } }
            div.stat-tile { div.label { "Retention (30d)" } div.value { (format!("{:.0}%", retention * 100.0)) } }
            div.stat-tile.maturity {
                div.label { "Maturity" }
                div.value {
                    "New " (new_n)
                    " · Learning " (learn_n)
                    " · Young " (young_n)
                    " · Mature " (mature_n)
                }
            }
        }
    })
}

fn per_deck_block(cards: &[Card], db: &Database, today: Date) -> Fallible<Markup> {
    let due = db.due_today(today)?;
    let mut totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for c in cards {
        let entry = totals.entry(c.deck_name().to_string()).or_insert((0, 0));
        entry.0 += 1;
        if due.contains(&c.hash()) {
            entry.1 += 1;
        }
    }
    Ok(html! {
        div.decks {
            h2 { "Decks" }
            table {
                thead { tr { th { "Deck" } th { "Due" } th { "Total" } } }
                tbody {
                    @for (deck, (total, due)) in &totals {
                        tr {
                            td { (deck) }
                            td { (due) }
                            td { (total) }
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helper::create_tmp_copy_of_test_directory;

    #[test]
    fn test_display_stats_format() {
        assert_eq!(StatsFormat::Html.to_string(), "html");
        assert_eq!(StatsFormat::Json.to_string(), "json");
    }

    #[test]
    fn test_print_stats_json() -> Fallible<()> {
        let dir = create_tmp_copy_of_test_directory()?;
        print_stats(Some(dir), StatsFormat::Json)?;
        Ok(())
    }

    #[test]
    fn test_print_stats_html() -> Fallible<()> {
        let dir = create_tmp_copy_of_test_directory()?;
        print_stats(Some(dir), StatsFormat::Html)?;
        Ok(())
    }

    #[test]
    fn test_get_stats() -> Fallible<()> {
        let dir = create_tmp_copy_of_test_directory()?;
        let coll = Collection::new(Some(dir))?;
        let stats = build_json_stats(&coll)?;
        assert!(stats.cards_in_deck_count > 0);
        Ok(())
    }
}
