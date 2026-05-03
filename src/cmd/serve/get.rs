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

use crate::cmd::serve::server::AnswerControls;
use crate::cmd::serve::state::MutableState;
use crate::cmd::serve::state::ServerState;
use crate::cmd::serve::template::page_template;
use crate::error::Fallible;
use crate::markdown::MarkdownRenderConfig;
use crate::media::resolve::MediaResolverBuilder;
use crate::types::card::Card;
use crate::types::card::CardType;

pub async fn get_handler(State(state): State<ServerState>) -> (StatusCode, Html<String>) {
    let html = match inner(state).await {
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

async fn inner(state: ServerState) -> Fallible<Markup> {
    let mutable = state.mutable.lock().unwrap();
    let body = if mutable.cards.is_empty() {
        render_caught_up()
    } else {
        render_session_page(&state, &mutable)?
    };
    let html = page_template(body);
    Ok(html)
}

fn render_caught_up() -> Markup {
    html! {
        div.caught-up {
            h1 { "You're caught up." }
            p { a href="/" { "← Dashboard" } }
        }
    }
}

fn render_session_page(state: &ServerState, mutable: &MutableState) -> Fallible<Markup> {
    let undo_disabled = mutable.reviews.is_empty();
    let total_cards = state.total_cards;
    let cards_done = state.total_cards - mutable.cards.len();
    let percent_done = (cards_done * 100).checked_div(total_cards).unwrap_or(100);
    let progress_bar_style = format!("width: {}%;", percent_done);
    let card = mutable.cards[0].clone();
    let coll_path = state.directory.clone();
    let deck_path = card.relative_file_path(&coll_path)?;
    let config = MarkdownRenderConfig {
        resolver: MediaResolverBuilder::new()
            .with_collection_path(coll_path)?
            .with_deck_path(deck_path)?
            .build()?,
        port: state.port,
    };
    let card_content = render_card(&card, mutable.reveal, &config)?;
    let card_controls = if mutable.reveal {
        let grades = match state.answer_controls {
            AnswerControls::Binary => html! {
                input id="forgot" type="submit" name="action" value="Forgot" title="Mark card as forgotten.";
                input id="good" type="submit" name="action" value="Good" title="Mark card as remembered.";
            },
            AnswerControls::Full => html! {
                input id="forgot" type="submit" name="action" value="Forgot" title="Mark card as forgotten. Shortcut: 1.";
                input id="hard" type="submit" name="action" value="Hard" title="Mark card as difficult. Shortcut: 2.";
                input id="good" type="submit" name="action" value="Good" title="Mark card as remembered well. Shortcut: 3.";
                input id="easy" type="submit" name="action" value="Easy" title="Mark card as very easy. Shortcut: 4.";
            },
        };
        html! {
            form action="/" method="post" {
                (undo_button(undo_disabled))
                div.spacer {}
                div.grades {
                    (grades)
                }
            }
        }
    } else {
        html! {
            form action="/" method="post" {
                (undo_button(undo_disabled))
                div.spacer {}
                input id="reveal" type="submit" name="action" value="Reveal" title="Show the answer. Shortcut: space.";
            }
        }
    };
    let html = html! {
        div.root {
            div.header {
                div.progress-bar {
                    div.progress-fill style=(progress_bar_style) {}
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

fn render_card(card: &Card, reveal: bool, config: &MarkdownRenderConfig) -> Fallible<Markup> {
    let html = match card.card_type() {
        CardType::Basic => {
            if reveal {
                html! {
                    div .question .rich-text {
                        (card.html_front(config)?)
                    }
                    div .answer .rich-text {
                        (card.html_back(config)?)
                    }
                }
            } else {
                html! {
                    div .question .rich-text {
                        (card.html_front(config)?)
                    }
                    div .answer .rich-text {}
                }
            }
        }
        CardType::Cloze => {
            if reveal {
                html! {
                    div .prompt .rich-text {
                        (card.html_back(config)?)
                    }
                }
            } else {
                html! {
                    div .prompt .rich-text {
                        (card.html_front(config)?)
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

fn undo_button(disabled: bool) -> Markup {
    if disabled {
        html! {
            input id="undo" type="submit" name="action" value="Undo" disabled;
        }
    } else {
        html! {
            input id="undo" type="submit" name="action" value="Undo" title="Undo last action. Shortcut: u.";
        }
    }
}

