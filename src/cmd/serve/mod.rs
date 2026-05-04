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

mod api;
mod dashboard;
mod get;
mod katex;
mod post;
pub mod server;
mod state;
pub mod template;
mod watcher;

#[cfg(test)]
mod tests {
    use std::fs::create_dir_all;

    use portpicker::pick_unused_port;
    use reqwest::StatusCode;
    use tempfile::tempdir;
    use tokio::spawn;

    use crate::cmd::serve::server::AnswerControls;
    use crate::cmd::serve::server::ServerConfig;
    use crate::cmd::serve::server::start_server;
    use crate::error::Fallible;
    use crate::helper::create_tmp_copy_of_test_directory;
    use crate::types::timestamp::Timestamp;
    use crate::utils::wait_for_server;

    const TEST_HOST: &str = "127.0.0.1";

    #[tokio::test]
    async fn test_start_server_on_non_existent_directory() -> Fallible<()> {
        let port = pick_unused_port().unwrap();
        let session_started_at = Timestamp::now();
        let config = ServerConfig {
            directory: Some("./derpherp".to_string()),
            host: TEST_HOST.to_string(),
            port,
            session_started_at,
            card_limit: None,
            new_card_limit: None,
            deck_filter: None,
            shuffle: false,
            answer_controls: AnswerControls::Full,
            bury_siblings: false,
            rescan_interval: None,
            no_watch: false,
        };
        let result = start_server(config).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert_eq!(err.to_string(), "error: directory does not exist.");
        Ok(())
    }

    #[tokio::test]
    async fn test_start_server_with_no_cards_due() -> Fallible<()> {
        let port = pick_unused_port().unwrap();
        let dir = tempdir()?.path().to_path_buf().canonicalize()?;
        create_dir_all(&dir)?;
        let session_started_at = Timestamp::now();
        let dir = dir.canonicalize().unwrap().display().to_string();
        let config = ServerConfig {
            directory: Some(dir),
            host: TEST_HOST.to_string(),
            port,
            session_started_at,
            card_limit: None,
            new_card_limit: None,
            deck_filter: None,
            shuffle: false,
            answer_controls: AnswerControls::Full,
            bury_siblings: false,
            rescan_interval: None,
            no_watch: false,
        };
        // Server starts (no early return when no cards due); spawn it and check
        // that the root endpoint returns the caught-up screen.
        spawn(async move { start_server(config).await });
        wait_for_server(TEST_HOST, port).await?;
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/")).await?;
        assert!(response.status().is_success());
        let html = response.text().await?;
        assert!(html.contains("You're caught up."));
        Ok(())
    }

    #[tokio::test]
    async fn test_e2e() -> Fallible<()> {
        let port = pick_unused_port().unwrap();
        let directory = create_tmp_copy_of_test_directory()?;
        let session_started_at = Timestamp::now();
        let config = ServerConfig {
            directory: Some(directory),
            host: TEST_HOST.to_string(),
            port,
            session_started_at,
            card_limit: None,
            new_card_limit: None,
            deck_filter: None,
            shuffle: false,
            answer_controls: AnswerControls::Full,
            bury_siblings: false,
            rescan_interval: None,
            no_watch: false,
        };
        spawn(async move { start_server(config).await });
        wait_for_server(TEST_HOST, port).await?;

        // Hit the `style.css` endpoint.
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/style.css")).await?;
        assert!(response.status().is_success());
        assert_eq!(response.headers().get("content-type").unwrap(), "text/css");

        // Hit the `script.js` endpoint.
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/script.js")).await?;
        assert!(response.status().is_success());
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/javascript"
        );

        // Hit the not found endpoint.
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/herp-derp")).await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Hit the file endpoint.
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/file/foo.jpg")).await?;
        assert!(response.status().is_success());
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "image/jpeg"
        );

        // Hit the file endpoint with a non-existent file.
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/file/foo.png")).await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Hit the root endpoint (dashboard).
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/")).await?;
        assert!(response.status().is_success());
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/html; charset=utf-8"
        );
        let html = response.text().await?;
        assert!(html.contains("cards due") || html.contains("caught up"));

        // GET /drill — both prompt halves are inlined; the answer is hidden
        // via class so JS can reveal it without another round-trip.
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/drill")).await?;
        assert!(response.status().is_success());
        let html = response.text().await?;
        assert!(html.contains("baz <span class='cloze-reveal'>quux</span>"));

        // POST 'Good' — accepted without any prior reveal action.
        let response = reqwest::Client::new()
            .post(format!("http://{TEST_HOST}:{port}/"))
            .form(&[("action", "Good")])
            .send()
            .await?;
        assert!(response.status().is_success());
        let html = response.text().await?;
        assert!(html.contains("FOO") && html.contains("BAR"));

        // Rate the final card.
        let response = reqwest::Client::new()
            .post(format!("http://{TEST_HOST}:{port}/"))
            .form(&[("action", "Good")])
            .send()
            .await?;
        assert!(response.status().is_success());
        let html = response.text().await?;
        assert!(html.contains("You're caught up."));

        Ok(())
    }

    #[tokio::test]
    async fn test_dashboard_renders() -> Fallible<()> {
        let port = pick_unused_port().unwrap();
        let directory = create_tmp_copy_of_test_directory()?;
        let session_started_at = Timestamp::now();
        let config = ServerConfig {
            directory: Some(directory),
            host: TEST_HOST.to_string(),
            port,
            session_started_at,
            card_limit: None,
            new_card_limit: None,
            deck_filter: None,
            shuffle: false,
            answer_controls: AnswerControls::Full,
            bury_siblings: false,
            rescan_interval: None,
            no_watch: true,
        };
        spawn(async move { start_server(config).await });
        wait_for_server(TEST_HOST, port).await?;
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/")).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let html = response.text().await?;
        assert!(html.contains("cards due") || html.contains("caught up"));
        Ok(())
    }

    #[tokio::test]
    async fn test_healthz() -> Fallible<()> {
        let port = pick_unused_port().unwrap();
        let directory = create_tmp_copy_of_test_directory()?;
        let session_started_at = Timestamp::now();
        let config = ServerConfig {
            directory: Some(directory),
            host: TEST_HOST.to_string(),
            port,
            session_started_at,
            card_limit: None,
            new_card_limit: None,
            deck_filter: None,
            shuffle: false,
            answer_controls: AnswerControls::Full,
            bury_siblings: false,
            rescan_interval: None,
            no_watch: true,
        };
        spawn(async move { start_server(config).await });
        wait_for_server(TEST_HOST, port).await?;
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/healthz")).await?;
        assert_eq!(response.status(), StatusCode::OK);
        Ok(())
    }

    #[tokio::test]
    async fn test_api_decks_returns_json() -> Fallible<()> {
        let port = pick_unused_port().unwrap();
        let directory = create_tmp_copy_of_test_directory()?;
        let session_started_at = Timestamp::now();
        let config = ServerConfig {
            directory: Some(directory),
            host: TEST_HOST.to_string(),
            port,
            session_started_at,
            card_limit: None,
            new_card_limit: None,
            deck_filter: None,
            shuffle: false,
            answer_controls: AnswerControls::Full,
            bury_siblings: false,
            rescan_interval: None,
            no_watch: true,
        };
        spawn(async move { start_server(config).await });
        wait_for_server(TEST_HOST, port).await?;
        let response = reqwest::get(format!("http://{TEST_HOST}:{port}/api/decks")).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let ct = response.headers().get("content-type").unwrap();
        assert!(ct.to_str().unwrap().contains("application/json"));
        Ok(())
    }

    #[tokio::test]
    async fn test_drill_deck_filter() -> Fallible<()> {
        let port = pick_unused_port().unwrap();
        let directory = create_tmp_copy_of_test_directory()?;
        let session_started_at = Timestamp::now();
        let config = ServerConfig {
            directory: Some(directory),
            host: TEST_HOST.to_string(),
            port,
            session_started_at,
            card_limit: None,
            new_card_limit: None,
            deck_filter: None,
            shuffle: false,
            answer_controls: AnswerControls::Full,
            bury_siblings: false,
            rescan_interval: None,
            no_watch: true,
        };
        spawn(async move { start_server(config).await });
        wait_for_server(TEST_HOST, port).await?;

        let response = reqwest::get(format!(
            "http://{TEST_HOST}:{port}/drill?deck=Deck"
        ))
        .await?;
        assert_eq!(response.status(), StatusCode::OK);
        Ok(())
    }

    /// Reveal is client-side: GET /drill must render BOTH the question and the
    /// answer text inline (the answer hidden via a CSS class), and the page
    /// must contain a "Reveal" control plus the grade buttons. POST grade
    /// works without any prior server-side reveal step. This rules out the
    /// bug class where POST→redirect→GET could re-shuffle and show a
    /// different card with the wrong answer.
    #[tokio::test]
    async fn test_drill_reveals_client_side_with_shuffle() -> Fallible<()> {
        use std::fs::write;

        let port = pick_unused_port().unwrap();
        let dir = tempdir()?.path().to_path_buf().canonicalize()?;
        create_dir_all(&dir)?;
        let cards = "\
Q: ALPHA-Q
A: ALPHA-A

Q: BRAVO-Q
A: BRAVO-A

Q: CHARLIE-Q
A: CHARLIE-A

Q: DELTA-Q
A: DELTA-A

Q: ECHO-Q
A: ECHO-A
";
        write(dir.join("Deck.md"), cards)?;
        let directory = dir.canonicalize().unwrap().display().to_string();

        let session_started_at = Timestamp::now();
        let config = ServerConfig {
            directory: Some(directory),
            host: TEST_HOST.to_string(),
            port,
            session_started_at,
            card_limit: None,
            new_card_limit: None,
            deck_filter: None,
            shuffle: true,
            answer_controls: AnswerControls::Full,
            bury_siblings: false,
            rescan_interval: None,
            no_watch: true,
        };
        spawn(async move { start_server(config).await });
        wait_for_server(TEST_HOST, port).await?;

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        let html1 = client
            .get(format!("http://{TEST_HOST}:{port}/drill"))
            .send()
            .await?
            .text()
            .await?;
        let fronts = ["ALPHA-Q", "BRAVO-Q", "CHARLIE-Q", "DELTA-Q", "ECHO-Q"];
        let backs = ["ALPHA-A", "BRAVO-A", "CHARLIE-A", "DELTA-A", "ECHO-A"];
        let chosen = fronts
            .iter()
            .position(|f| html1.contains(f))
            .expect("first GET /drill should render one of the known cards");
        let chosen_front = fronts[chosen];
        let chosen_back = backs[chosen];

        // Both halves must be in the DOM so JS can flip visibility client-side.
        assert!(
            html1.contains(chosen_front),
            "question {chosen_front} should be rendered"
        );
        assert!(
            html1.contains(chosen_back),
            "answer {chosen_back} should be inlined (hidden via class) for client-side reveal"
        );
        // The Reveal control and grade buttons must all be present.
        assert!(html1.contains(r#"id="reveal""#), "Reveal button missing");
        assert!(html1.contains(r#"id="good""#), "Good button missing");
        assert!(
            html1.contains("is-hidden"),
            "answer/grades should be hidden via the is-hidden class"
        );

        // POST a grade with no prior Reveal — it should be accepted (the
        // server no longer gates ratings on a reveal flag).
        let resp = client
            .post(format!("http://{TEST_HOST}:{port}/"))
            .form(&[("action", "Good")])
            .send()
            .await?;
        assert!(resp.status().is_redirection());

        // The next GET cycles in a different card.
        let html2 = client
            .get(format!("http://{TEST_HOST}:{port}/drill"))
            .send()
            .await?
            .text()
            .await?;
        assert!(
            !html2.contains(chosen_front) || html2.contains("You're caught up."),
            "after grading, the chosen card {chosen_front} should not be the one rendered"
        );
        Ok(())
    }
}
