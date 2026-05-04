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

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::cmd::serve::server::AnswerControls;
use crate::cmd::serve::server::bury_siblings;
use crate::cmd::serve::server::filter_deck;
use crate::db::Database;
use crate::error::Fallible;
use crate::rng::TinyRng;
use crate::rng::shuffle;
use crate::types::card::Card;
use crate::types::card_hash::CardHash;
use crate::types::date::Date;

pub struct CardIndex {
    pub cards: Vec<Card>,
}

#[derive(Clone)]
pub struct ServeFilters {
    pub card_limit: Option<usize>,
    pub new_card_limit: Option<usize>,
    pub deck_filter: Option<String>,
    pub bury_siblings: bool,
    pub shuffle: bool,
}

#[derive(Clone)]
pub struct SessionState {
    /// Cards that were rated Forgot/Hard this session and must be re-shown
    /// before any other due card, because MIN_INTERVAL=1 means they are
    /// scheduled for tomorrow in the DB (not today).
    pub relapse_queue: Vec<CardHash>,
    /// The hash of the card most recently rendered to the user by GET /drill.
    /// POST /rate reads this to grade the card the user actually saw, avoiding
    /// a race where compute_due_queue re-shuffles between the GET and POST.
    pub current_card: Option<CardHash>,
}

#[derive(Clone)]
pub struct AppState {
    pub port: u16,
    pub directory: PathBuf,
    pub macros: Vec<(String, String)>,
    pub session_id: i64,
    pub answer_controls: AnswerControls,
    pub filters: ServeFilters,
    pub cards: Arc<RwLock<CardIndex>>,
    pub db: Arc<Mutex<Database>>,
    pub session_state: Arc<Mutex<SessionState>>,
}

impl AppState {
    /// Build the live queue of due cards according to the configured filters.
    /// Relapsed cards (Forgot/Hard) are prepended from `session_state`.
    /// Caller holds no locks; this acquires read on cards and lock on db.
    pub fn compute_due_queue(&self, today: Date) -> Fallible<Vec<Card>> {
        // Snapshot relapse queue first to keep a consistent lock order downstream
        // (cards -> db). Holding session_state while acquiring cards would invert
        // the order and create a latent deadlock once the file watcher takes a
        // write lock on cards.
        let relapse_hashes: Vec<CardHash> = {
            let session = self.session_state.lock().unwrap();
            session.relapse_queue.clone()
        };

        let index = self.cards.read().unwrap();
        let all_cards = index.cards.clone();
        drop(index);

        let db = self.db.lock().unwrap();
        let due_today: HashSet<CardHash> = db.due_today(today)?;
        let rejected: HashSet<CardHash> = db.rejected_card_hashes()?;
        drop(db);

        let due_today: Vec<Card> = all_cards
            .iter()
            .filter(|c| due_today.contains(&c.hash()) && !rejected.contains(&c.hash()))
            .cloned()
            .collect();

        let db = self.db.lock().unwrap();
        let mut due_today = filter_deck(
            &db,
            due_today,
            self.filters.card_limit,
            self.filters.new_card_limit,
            self.filters.deck_filter.clone(),
        )?;
        drop(db);

        if self.filters.bury_siblings {
            due_today = bury_siblings(due_today);
        }

        if self.filters.shuffle {
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64;
            let mut rng = TinyRng::from_seed(seed);
            due_today = shuffle(due_today, &mut rng);
        }

        // Prepend relapsed cards (Forgot/Hard rated this session), using the
        // snapshot taken at the top of this method.
        if !relapse_hashes.is_empty() {
            // Build relapse cards in order, deduplicating against due_today
            // and dropping anything the user has rejected since.
            let due_hashes: HashSet<CardHash> = due_today.iter().map(|c| c.hash()).collect();
            let mut relapse_cards: Vec<Card> = relapse_hashes
                .iter()
                .filter_map(|hash| {
                    if due_hashes.contains(hash) || rejected.contains(hash) {
                        None
                    } else {
                        all_cards.iter().find(|c| &c.hash() == hash).cloned()
                    }
                })
                .collect();
            relapse_cards.extend(due_today);
            return Ok(relapse_cards);
        }

        Ok(due_today)
    }
}
