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

use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use maud::Markup;
use maud::html;
use serde::Deserialize;

use crate::cmd::serve::server::AnswerControls;
use crate::cmd::serve::state::AppState;
use crate::cmd::serve::template::page_template;
use crate::error::Fallible;
use crate::markdown::MarkdownRenderConfig;
use crate::media::resolve::MediaResolverBuilder;
use crate::types::card::Card;
use crate::types::card::CardType;
use crate::types::date::Date;

#[derive(Deserialize)]
pub struct DrillQuery {
    pub deck: Option<String>,
}

pub async fn get_handler(
    State(state): State<AppState>,
    Query(q): Query<DrillQuery>,
) -> (StatusCode, Html<String>) {
    let html = match inner(state, q.deck).await {
        Ok(html) => html,
        Err(e) => page_template(html! {
            div.error {
                h1 { "Error" }
                p { (e) }
            }
        }),
    };
    (StatusCode::OK, Html(html.into_string()))
}

async fn inner(state: AppState, deck: Option<String>) -> Fallible<Markup> {
    let today = Date::today();
    let mut filtered_state = state.clone();
    if let Some(d) = deck {
        filtered_state.filters.deck_filter = Some(d);
    }
    let queue = filtered_state.compute_due_queue(today)?;
    let body = if queue.is_empty() {
        let mut session = filtered_state.session_state.lock().unwrap();
        session.current_card = None;
        drop(session);
        render_caught_up()
    } else {
        let remaining = queue.len();
        // Pin to the previously-rendered card if it's still in the queue, so a
        // page refresh doesn't re-shuffle out from under the user. POST clears
        // current_card on grade, so the next GET cycles in fresh.
        let pinned_hash = filtered_state.session_state.lock().unwrap().current_card;
        let card = pinned_hash
            .and_then(|h| queue.iter().find(|c| c.hash() == h).cloned())
            .unwrap_or_else(|| queue.first().cloned().unwrap());
        filtered_state.session_state.lock().unwrap().current_card = Some(card.hash());
        render_card_page(&filtered_state, &card, remaining)?
    };
    Ok(page_template(body))
}

fn render_caught_up() -> Markup {
    html! {
        div.caught-up {
            h1 { "You're caught up." }
            p { a href="/" { "← Dashboard" } }
        }
    }
}

fn render_card_page(
    state: &AppState,
    card: &Card,
    remaining: usize,
) -> Fallible<Markup> {
    let coll_path = state.directory.clone();
    let deck_path = card.relative_file_path(&coll_path)?;
    let config = MarkdownRenderConfig {
        resolver: MediaResolverBuilder::new()
            .with_collection_path(coll_path)?
            .with_deck_path(deck_path)?
            .build()?,
        port: state.port,
    };
    let card_content = render_card(card, &config)?;
    let grades = match state.answer_controls {
        AnswerControls::Binary => html! {
            input id="forgot" type="submit" name="action" value="Forgot" title="Mark card as forgotten." disabled;
            input id="good" type="submit" name="action" value="Good" title="Mark card as remembered." disabled;
            input id="reject" .reject type="submit" name="action" value="Reject" title="Reject this card as low quality. Shortcut: r." disabled;
        },
        AnswerControls::Full => html! {
            input id="forgot" type="submit" name="action" value="Forgot" title="Mark card as forgotten. Shortcut: 1." disabled;
            input id="hard" type="submit" name="action" value="Hard" title="Mark card as difficult. Shortcut: 2." disabled;
            input id="good" type="submit" name="action" value="Good" title="Mark card as remembered well. Shortcut: 3." disabled;
            input id="easy" type="submit" name="action" value="Easy" title="Mark card as very easy. Shortcut: 4." disabled;
            input id="reject" .reject type="submit" name="action" value="Reject" title="Reject this card as low quality. It won't appear again. Shortcut: r." disabled;
        },
    };
    // Reveal is a client-side toggle: clicking it unhides the answer and the
    // grade buttons. No POST, no GET, no chance for the queue to re-shuffle
    // out from under the user.
    let card_controls = html! {
        form action="/" method="post" {
            div.spacer {}
            input id="reveal" type="button" value="Reveal" title="Show the answer. Shortcut: space.";
            div.grades.is-hidden {
                (grades)
            }
            div.spacer {}
        }
    };
    let html = html! {
        div.root {
            div.header {
                div.due-count {
                    (remaining) " due "
                    a.dashboard-link href="/" { "← Dashboard" }
                }
            }
            div.card-container {
                div.card {
                    div.card-header {
                        h1 {
                            (card.deck_name())
                        }
                    }
                    (card_content)
                }
            }
            div.controls {
                (card_controls)
            }
        }
    };
    Ok(html)
}

fn render_card(card: &Card, config: &MarkdownRenderConfig) -> Fallible<Markup> {
    let html = match card.card_type() {
        CardType::Basic => {
            html! {
                div .question .rich-text {
                    (card.html_front(config)?)
                }
                div .answer .rich-text {
                    div #answer-body .is-hidden {
                        (card.html_back(config)?)
                    }
                }
            }
        }
        CardType::Cloze => {
            html! {
                div .prompt .rich-text {
                    div #prompt-front {
                        (card.html_front(config)?)
                    }
                    div #prompt-back .is-hidden {
                        (card.html_back(config)?)
                    }
                }
            }
        }
    };
    Ok(html! {
        div.card-content {
            (html)
        }
    })
}
