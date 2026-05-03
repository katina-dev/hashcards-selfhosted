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
use crate::cmd::serve::state::AppState;
use crate::cmd::serve::state::SessionState;
use crate::cmd::serve::template::page_template;
use crate::error::Fallible;
use crate::markdown::MarkdownRenderConfig;
use crate::media::resolve::MediaResolverBuilder;
use crate::types::card::Card;
use crate::types::card::CardType;
use crate::types::date::Date;

pub async fn get_handler(State(state): State<AppState>) -> (StatusCode, Html<String>) {
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

async fn inner(state: AppState) -> Fallible<Markup> {
    let today = Date::today();
    let queue = state.compute_due_queue(today)?;
    let session = state.session_state.lock().unwrap();
    let body = if queue.is_empty() {
        render_caught_up()
    } else {
        let remaining = queue.len();
        let card = queue.into_iter().next().unwrap();
        render_card_page(&state, &session, &card, remaining)?
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
    session: &SessionState,
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
    let card_content = render_card(card, session.reveal, &config)?;
    let card_controls = if session.reveal {
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
                div.spacer {}
                div.grades {
                    (grades)
                }
                div.spacer {}
            }
        }
    } else {
        html! {
            form action="/" method="post" {
                div.spacer {}
                input id="reveal" type="submit" name="action" value="Reveal" title="Show the answer. Shortcut: space.";
                div.spacer {}
            }
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
