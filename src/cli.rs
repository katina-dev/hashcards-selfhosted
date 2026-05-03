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

use clap::Parser;
use clap::Subcommand;

use crate::cmd::check::check_collection;
use crate::cmd::serve::server::AnswerControls;
use crate::cmd::serve::server::ServerConfig;
use crate::cmd::serve::server::start_server;
use crate::cmd::export::export_collection;
use crate::cmd::orphans::delete_orphans;
use crate::cmd::orphans::list_orphans;
use crate::cmd::stats::StatsFormat;
use crate::cmd::stats::print_stats;
use crate::error::Fallible;
use crate::types::timestamp::Timestamp;

#[derive(Parser)]
#[command(version, about, long_about = None)]
enum Command {
    /// Serve cards through a long-running web interface for self-hosting.
    Serve {
        /// Path to the collection directory. By default, the current working directory is used.
        #[arg(env = "HASHCARDS_DIRECTORY")]
        directory: Option<String>,
        /// Maximum number of cards to drill in a session. By default, all cards due are drilled.
        #[arg(long, env = "HASHCARDS_CARD_LIMIT")]
        card_limit: Option<usize>,
        /// Maximum number of new cards to drill in a session.
        #[arg(long, env = "HASHCARDS_NEW_CARD_LIMIT")]
        new_card_limit: Option<usize>,
        /// The host address to bind to. Default is 0.0.0.0 for container friendliness.
        #[arg(long, default_value = "0.0.0.0", env = "HASHCARDS_HOST")]
        host: String,
        /// The port to use for the web server. Default is 8000.
        #[arg(long, default_value_t = 8000, env = "HASHCARDS_PORT")]
        port: u16,
        /// Only drill cards from this deck.
        #[arg(long, env = "HASHCARDS_FROM_DECK")]
        from_deck: Option<String>,
        /// Which answer controls to show.
        #[arg(long, default_value_t = AnswerControls::Full, env = "HASHCARDS_ANSWER_CONTROLS")]
        answer_controls: AnswerControls,
        /// Whether or not to bury siblings. Default is true.
        #[arg(long, env = "HASHCARDS_BURY_SIBLINGS")]
        bury_siblings: Option<bool>,
        /// Polling rescan interval (e.g. "30s") for filesystems where inotify is silent.
        #[arg(long, env = "HASHCARDS_RESCAN_INTERVAL")]
        rescan_interval: Option<String>,
        /// Disable the inotify-based file watcher.
        #[arg(long, env = "HASHCARDS_NO_WATCH")]
        no_watch: bool,
    },
    /// Check the integrity of a collection.
    Check {
        /// Path to the collection directory. By default, the current working directory is used.
        directory: Option<String>,
    },
    /// Print collection statistics.
    Stats {
        /// Path to the collection directory. By default, the current working directory is used.
        directory: Option<String>,
        /// Which output format to use.
        #[arg(long, default_value_t = StatsFormat::Html)]
        format: StatsFormat,
    },
    /// Commands relating to orphan cards.
    Orphans {
        #[command(subcommand)]
        command: OrphanCommand,
    },
    /// Export a collection.
    Export {
        /// Path to the collection directory. By default, the current working directory is used.
        directory: Option<String>,
        /// Optional path to the output file. By default, the output is printed to stdout.
        #[arg(long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum OrphanCommand {
    /// List the hashes of all orphan cards in the collection.
    List {
        /// Path to the collection directory. By default, the current working directory is used.
        directory: Option<String>,
    },
    /// Remove all orphan cards from the database.
    Delete {
        /// Path to the collection directory. By default, the current working directory is used.
        directory: Option<String>,
    },
}

pub async fn entrypoint() -> Fallible<()> {
    let cli: Command = Command::parse();
    match cli {
        Command::Serve {
            directory,
            card_limit,
            new_card_limit,
            host,
            port,
            from_deck,
            answer_controls,
            bury_siblings,
            rescan_interval,
            no_watch,
        } => {
            let config = ServerConfig {
                directory,
                host,
                port,
                session_started_at: Timestamp::now(),
                card_limit,
                new_card_limit,
                deck_filter: from_deck,
                shuffle: true,
                answer_controls,
                bury_siblings: bury_siblings.unwrap_or(true),
                rescan_interval,
                no_watch,
            };
            start_server(config).await
        }
        Command::Check { directory } => check_collection(directory),
        Command::Stats { directory, format } => print_stats(directory, format),
        Command::Orphans { command } => match command {
            OrphanCommand::List { directory } => list_orphans(directory),
            OrphanCommand::Delete { directory } => delete_orphans(directory),
        },
        Command::Export { directory, output } => export_collection(directory, output),
    }
}
