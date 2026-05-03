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
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

use axum::Router;
use axum::extract::Path;
use axum::extract::State;
use axum::http::HeaderName;
use axum::http::StatusCode;
use axum::http::header::CACHE_CONTROL;
use axum::http::header::CONTENT_TYPE;
use axum::response::Html;
use axum::routing::get;
use axum::routing::post;
use clap::ValueEnum;
use tokio::net::TcpListener;
use tokio::select;

use crate::cmd::serve::get::get_handler;
use crate::cmd::serve::katex::KATEX_CSS_URL;
use crate::cmd::serve::katex::KATEX_JS_URL;
use crate::cmd::serve::katex::KATEX_MHCHEM_JS_URL;
use crate::cmd::serve::katex::katex_css_handler;
use crate::cmd::serve::katex::katex_font_handler;
use crate::cmd::serve::katex::katex_js_handler;
use crate::cmd::serve::katex::katex_mhchem_js_handler;
use crate::cmd::serve::post::post_handler;
use crate::cmd::serve::state::AppState;
use crate::cmd::serve::state::CardIndex;
use crate::cmd::serve::state::ServeFilters;
use crate::cmd::serve::state::SessionState;
use crate::collection::Collection;
use crate::db::Database;
use crate::error::Fallible;
use crate::error::fail;
use crate::media::load::MediaLoader;
use crate::types::card::Card;
use crate::types::card_hash::CardHash;
use crate::types::timestamp::Timestamp;
use crate::utils::CACHE_CONTROL_IMMUTABLE;

#[derive(ValueEnum, Clone, Copy, PartialEq)]
pub enum AnswerControls {
    /// Show all four rating buttons (Forgot/Hard/Good/Easy).
    Full,
    /// Show only two rating buttons (Forgot/Good).
    Binary,
}

impl Display for AnswerControls {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AnswerControls::Full => write!(f, "full"),
            AnswerControls::Binary => write!(f, "binary"),
        }
    }
}

pub struct ServerConfig {
    pub directory: Option<String>,
    pub host: String,
    pub port: u16,
    pub session_started_at: Timestamp,
    pub card_limit: Option<usize>,
    pub new_card_limit: Option<usize>,
    pub deck_filter: Option<String>,
    pub shuffle: bool,
    pub answer_controls: AnswerControls,
    pub bury_siblings: bool,
    pub rescan_interval: Option<String>,
    pub no_watch: bool,
}

pub async fn start_server(config: ServerConfig) -> Fallible<()> {
    let Collection {
        directory,
        mut db,
        cards,
        macros,
    } = Collection::new(config.directory)?;

    let db_hashes: HashSet<CardHash> = db.card_hashes()?;
    for card in cards.iter() {
        if !db_hashes.contains(&card.hash()) {
            db.insert_card(card.hash(), config.session_started_at)?;
        }
    }

    let session_id: i64 = db.create_session(config.session_started_at)?;

    let card_index = CardIndex { cards };
    let state = AppState {
        port: config.port,
        directory,
        macros,
        session_id,
        answer_controls: config.answer_controls,
        filters: ServeFilters {
            card_limit: config.card_limit,
            new_card_limit: config.new_card_limit,
            deck_filter: config.deck_filter,
            bury_siblings: config.bury_siblings,
            shuffle: config.shuffle,
        },
        cards: Arc::new(RwLock::new(card_index)),
        db: Arc::new(Mutex::new(db)),
        session_state: Arc::new(Mutex::new(SessionState {
            reveal: false,
            relapse_queue: Vec::new(),
        })),
    };

    let rescan_interval = match config.rescan_interval.as_deref() {
        Some(s) => Some(parse_duration(s)?),
        None => None,
    };
    crate::cmd::serve::watcher::spawn_watcher(
        state.directory.clone(),
        state.cards.clone(),
        state.db.clone(),
        rescan_interval,
        !config.no_watch,
    )?;

    let app = Router::new()
        .route("/", get(get_handler))
        .route("/", post(post_handler))
        .route("/script.js", get(script_handler))
        .route("/style.css", get(style_handler))
        .route("/favicon.ico", get(favicon_handler))
        .route(KATEX_CSS_URL, get(katex_css_handler))
        .route(KATEX_JS_URL, get(katex_js_handler))
        .route(KATEX_MHCHEM_JS_URL, get(katex_mhchem_js_handler))
        .route("/katex/fonts/{*path}", get(katex_font_handler))
        .route("/file/{*path}", get(file_handler))
        .fallback(not_found_handler)
        .with_state(state);

    let bind = format!("{}:{}", config.host, config.port);
    log::info!("hashcards serve listening on {bind}");
    let listener = TcpListener::bind(bind).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn script_handler(
    State(state): State<AppState>,
) -> (StatusCode, [(HeaderName, &'static str); 1], String) {
    let mut content = String::new();
    content.push_str("let MACROS = {};\n");
    for (name, definition) in &state.macros {
        let name = escape_js_string_literal(name);
        let definition = escape_js_string_literal(definition);
        content.push_str(&format!("MACROS['{name}'] = '{definition}';\n"));
    }
    content.push('\n');
    content.push_str(include_str!("script.js"));
    (StatusCode::OK, [(CONTENT_TYPE, "text/javascript")], content)
}

fn escape_js_string_literal(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('$', "\\$")
}

async fn style_handler() -> (StatusCode, [(HeaderName, &'static str); 2], &'static [u8]) {
    let bytes = include_bytes!("style.css");
    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, "text/css"),
            (CACHE_CONTROL, CACHE_CONTROL_IMMUTABLE),
        ],
        bytes,
    )
}

async fn favicon_handler() -> (StatusCode, [(HeaderName, &'static str); 2], &'static [u8]) {
    let bytes = include_bytes!("favicon.png");
    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, "image/png"),
            (CACHE_CONTROL, CACHE_CONTROL_IMMUTABLE),
        ],
        bytes,
    )
}

async fn not_found_handler() -> (StatusCode, Html<String>) {
    (StatusCode::NOT_FOUND, Html("Not Found".to_string()))
}

async fn file_handler(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> (StatusCode, [(HeaderName, &'static str); 1], Vec<u8>) {
    let loader = MediaLoader::new(state.directory.clone());
    let validated_path: PathBuf = match loader.validate(&path) {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                [(CONTENT_TYPE, "text/plain")],
                b"Not Found".to_vec(),
            );
        }
    };
    let extension = validated_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content_type: &str = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    };
    let content = tokio::fs::read(validated_path).await;
    match content {
        Ok(bytes) => (StatusCode::OK, [(CONTENT_TYPE, content_type)], bytes),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(CONTENT_TYPE, "text/plain")],
            b"Internal Server Error".to_vec(),
        ),
    }
}

async fn shutdown_signal() {
    use tokio::signal::unix::SignalKind;
    use tokio::signal::unix::signal;

    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let term = async {
        signal(SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let term = std::future::pending::<()>();

    select! {
        _ = ctrl_c => log::info!("Received Ctrl+C, shutting down"),
        _ = term => log::info!("Received SIGTERM, shutting down"),
    }
}

pub fn filter_deck(
    db: &Database,
    deck: Vec<Card>,
    card_limit: Option<usize>,
    new_card_limit: Option<usize>,
    deck_filter: Option<String>,
) -> Fallible<Vec<Card>> {
    // Apply the deck filter.
    let deck = match deck_filter {
        Some(filter) => deck
            .into_iter()
            .filter(|card| card.deck_name() == &filter)
            .collect(),
        None => deck,
    };

    // Apply the card limit.
    let deck = match card_limit {
        Some(limit) => deck.into_iter().take(limit).collect(),
        None => deck,
    };

    // Apply the new card limit.
    let deck = match new_card_limit {
        Some(limit) => {
            let mut new_count = 0;
            let mut result = Vec::new();
            for card in deck.into_iter() {
                if db.get_card_performance(card.hash())?.is_new() {
                    if new_count < limit {
                        result.push(card);
                        new_count += 1;
                    }
                } else {
                    result.push(card);
                }
            }
            result
        }
        None => deck,
    };

    Ok(deck)
}

pub fn bury_siblings(deck: Vec<Card>) -> Vec<Card> {
    let mut seen_families = HashSet::new();
    let mut result = Vec::new();
    for card in deck.into_iter() {
        if let Some(family) = card.family_hash() {
            if seen_families.contains(&family) {
                continue;
            }
            seen_families.insert(family);
        }
        result.push(card);
    }
    result
}

fn parse_duration(s: &str) -> Fallible<std::time::Duration> {
    let s = s.trim();
    let (num_part, unit) = s
        .find(|c: char| c.is_alphabetic())
        .map(|i| (&s[..i], &s[i..]))
        .unwrap_or((s, "s"));
    let n: u64 = num_part.parse().map_err(|_| {
        crate::error::ErrorReport::new(format!("invalid duration: {s}"))
    })?;
    let secs = match unit {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        other => return fail(format!("invalid duration unit: {other}")),
    };
    Ok(std::time::Duration::from_secs(secs))
}
