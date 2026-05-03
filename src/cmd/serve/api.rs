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

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;

use crate::cmd::serve::state::AppState;
use crate::types::date::Date;

#[derive(Serialize)]
pub struct DeckCounts {
    pub due_count: usize,
    pub total_count: usize,
}

#[derive(Serialize)]
pub struct CardSummary {
    pub hash: String,
    pub deck: String,
    pub source_file: String,
}

pub async fn healthz_handler() -> StatusCode {
    StatusCode::OK
}

pub async fn api_decks_handler(
    State(state): State<AppState>,
) -> (StatusCode, Json<BTreeMap<String, DeckCounts>>) {
    match build_deck_counts(&state) {
        Ok(map) => (StatusCode::OK, Json(map)),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(BTreeMap::new())),
    }
}

fn build_deck_counts(state: &AppState) -> crate::error::Fallible<BTreeMap<String, DeckCounts>> {
    let cards = state.cards.read().unwrap().cards.clone();
    let today = Date::today();
    let due = state.db.lock().unwrap().due_today(today)?;

    let mut map: BTreeMap<String, DeckCounts> = BTreeMap::new();
    for card in &cards {
        let deck = card.deck_name().to_string();
        let entry = map.entry(deck).or_insert(DeckCounts {
            due_count: 0,
            total_count: 0,
        });
        entry.total_count += 1;
        if due.contains(&card.hash()) {
            entry.due_count += 1;
        }
    }
    Ok(map)
}

pub async fn api_cards_handler(
    State(state): State<AppState>,
) -> (StatusCode, Json<Vec<CardSummary>>) {
    let cards = state.cards.read().unwrap().cards.clone();
    let dir = state.directory.clone();
    let mut out: Vec<CardSummary> = Vec::with_capacity(cards.len());
    for c in &cards {
        let rel = c
            .relative_file_path(&dir)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        out.push(CardSummary {
            hash: c.hash().to_hex(),
            deck: c.deck_name().to_string(),
            source_file: rel,
        });
    }
    (StatusCode::OK, Json(out))
}
