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

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use notify::Config;
use notify::EventKind;
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify::Watcher;
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio::time::sleep;

use crate::cmd::serve::state::CardIndex;
use crate::collection::Collection;
use crate::db::Database;
use crate::error::Fallible;
use crate::types::timestamp::Timestamp;

const DEBOUNCE: Duration = Duration::from_millis(400);

/// Spawn a file-watcher task that rebuilds the card index whenever .md files
/// change. Returns immediately; the watcher runs in the background.
///
/// `rescan_interval`: optional polling fallback for filesystems where
/// inotify is silent (NFS, some Docker Desktop setups, Windows host paths).
/// `enable_watch`: when false, only the polling fallback runs (or nothing).
pub fn spawn_watcher(
    directory: PathBuf,
    cards: Arc<RwLock<CardIndex>>,
    db: Arc<std::sync::Mutex<Database>>,
    rescan_interval: Option<Duration>,
    enable_watch: bool,
) -> Fallible<()> {
    let (tx, mut rx) = mpsc::channel::<()>(16);

    if enable_watch {
        let tx_for_notify = tx.clone();
        let mut watcher: RecommendedWatcher = match RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    if event_is_relevant(&event) {
                        let _ = tx_for_notify.blocking_send(());
                    }
                }
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                if rescan_interval.is_none() {
                    return Err(crate::error::ErrorReport::new(format!(
                        "failed to set up file watcher: {e}. Use --rescan-interval to enable polling, or --no-watch to disable."
                    )));
                }
                log::warn!("file watcher unavailable ({e}); falling back to polling.");
                spawn_poll_only(directory, cards, db, rescan_interval.unwrap());
                return Ok(());
            }
        };
        watcher.watch(&directory, RecursiveMode::Recursive)
            .map_err(|e| crate::error::ErrorReport::new(format!("watch failed: {e}")))?;
        // Keep the watcher alive for the program's lifetime.
        std::mem::forget(watcher);
    }

    if let Some(every) = rescan_interval {
        let tx_poll = tx.clone();
        tokio::spawn(async move {
            let mut tick = interval(every);
            loop {
                tick.tick().await;
                let _ = tx_poll.send(()).await;
            }
        });
    }

    tokio::spawn(async move {
        loop {
            if rx.recv().await.is_none() {
                break;
            }
            sleep(DEBOUNCE).await;
            while rx.try_recv().is_ok() {}
            if let Err(e) = rebuild_index(&directory, &cards, &db) {
                log::error!("watcher rebuild failed: {e}");
            }
        }
    });

    Ok(())
}

fn spawn_poll_only(
    directory: PathBuf,
    cards: Arc<RwLock<CardIndex>>,
    db: Arc<std::sync::Mutex<Database>>,
    every: Duration,
) {
    tokio::spawn(async move {
        let mut tick = interval(every);
        loop {
            tick.tick().await;
            if let Err(e) = rebuild_index(&directory, &cards, &db) {
                log::error!("polling rebuild failed: {e}");
            }
        }
    });
}

fn event_is_relevant(event: &notify::Event) -> bool {
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return false;
    }
    event.paths.iter().any(|p| is_md(p))
}

fn is_md(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()) == Some("md")
}

fn rebuild_index(
    directory: &PathBuf,
    cards: &Arc<RwLock<CardIndex>>,
    db: &Arc<std::sync::Mutex<Database>>,
) -> Fallible<()> {
    let (new_cards, _macros) = Collection::parse_only(directory)?;
    let now = Timestamp::now();
    {
        let db = db.lock().unwrap();
        let known = db.card_hashes()?;
        for c in &new_cards {
            if !known.contains(&c.hash()) {
                db.insert_card(c.hash(), now)?;
            }
        }
    }
    let mut guard = cards.write().unwrap();
    guard.cards = new_cards;
    log::info!("card index rebuilt ({} cards)", guard.cards.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::write;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_watcher_picks_up_new_md_file() -> Fallible<()> {
        let dir = tempdir()?;
        let dir_path = dir.path().to_path_buf();

        let db_path = dir_path.join("hashcards.db");
        let db = Database::new(db_path.to_str().unwrap())?;
        let cards = Arc::new(RwLock::new(CardIndex { cards: vec![] }));
        let db = Arc::new(Mutex::new(db));

        spawn_watcher(dir_path.clone(), cards.clone(), db.clone(), None, true)?;

        write(
            dir_path.join("Test.md"),
            "Q: foo\nA: bar\n",
        )?;

        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if cards.read().unwrap().cards.len() > 0 {
                return Ok(());
            }
        }
        panic!("watcher did not rebuild within 2s");
    }
}
