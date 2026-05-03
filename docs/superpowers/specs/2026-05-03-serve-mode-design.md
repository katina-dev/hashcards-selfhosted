# `serve` mode: perpetual self-hosted hashcards

**Status:** Design — approved 2026-05-03
**Author:** brainstormed with Claude

## Goal

Turn hashcards into an application that can be perpetually self-hosted as a Docker container. The user visits a URL in any browser (desktop or mobile), sees what's due, and drills cards. Progress is saved per-rating and survives tab closes, device switches, and container restarts. Card files are edited on the host filesystem and picked up automatically by the running service.

## Non-goals

- Multi-user accounts or authentication. The single user secures network access externally (tailnet/VPN/LAN).
- LLM-assisted card authoring. Deferred to a separate sub-project; this work exposes a small read-only HTTP API to make a future LLM tool straightforward to build, but builds no LLM client itself.
- A web upload form for cards. Cards remain plain text files edited with the user's tools of choice; the file system is the API.
- Publishing Docker images to ghcr.io / Docker Hub, multi-arch builds, Kubernetes manifests. Out of scope; users build the image themselves initially.
- Replacing FSRS, the parser, the markdown renderer, or any other existing core component.

## Summary of decisions

| Decision | Choice |
|---|---|
| Audience | Single user, no in-app auth |
| Persistence | Per-rating; immediate writes to SQLite |
| File pickup | Auto via `notify` crate, `--rescan-interval` polling fallback |
| CLI shape | Replace `drill` with `serve`; `drill` is removed |
| Landing experience | Dashboard first; "Drill" button leads into card view |
| LLM integration | Deferred; expose read-only `/api/decks` and `/api/cards` only |
| Deployment target | Docker (primary and only first-class) |

## Architecture

### Module structure

```
src/cmd/serve/         (renamed/rebuilt from src/cmd/drill/)
  mod.rs
  server.rs            HTTP server bootstrap; route table
  state.rs             AppState: card index + DB handle + config
  watcher.rs           NEW. notify-based file watcher + debounce
  dashboard.rs         NEW. GET / handler, stats aggregation
  get.rs               GET /drill, GET /file/*, etc.
  post.rs              POST /rate (per-rating persistence)
  template.rs          Existing template module
  static assets        style.css, script.js, katex.rs, media/, favicon.png
```

`src/cmd/drill/cache.rs` is **deleted**. Its role disappears under per-rating persistence; the in-memory state becomes a read-side card index only.

### CLI surface

`hashcards serve [opts] [directory]`. The `drill` subcommand is removed from `cli.rs`.

Other commands (`check`, `stats`, `orphans`, `export`) are unchanged. The `stats --format html` output, currently stubbed, is implemented as part of this work using the same template fragments as the dashboard.

### `AppState`

Held in an `Arc`, shared across handlers:

- `cards: RwLock<CardIndex>` — parsed-card index; map from card hash to card content, deck name, and source file. Rebuilt by the watcher on file changes.
- `db: Database` — SQLite handle. Concurrency model follows the existing pattern in `db.rs`.
- `config: ServeConfig` — directory path, deck filter, card limits, watcher mode.

### Persistence model

Every rating writes through to SQLite immediately, in a single transaction:

1. Validate the card hash exists in `cards`.
2. Compute new FSRS state via `fsrs.rs`.
3. In one transaction: insert review record, update card performance.
4. Return next due card (or "caught up" sentinel) as a JSON payload.

No buffering. No "session" concept user-visible. A crash mid-drill loses at most the in-flight rating.

The CLAUDE.md rule "Don't persist changes to the database during drilling. Use the cache." is **removed** as part of this work and replaced with: "Serve mode persists ratings per-card. The in-memory state is a read-side index only; never use it as a write buffer."

### Concurrent device behavior

Two devices can both have the page open simultaneously. There is no locking. Last-write-wins on accidental simultaneous ratings of the same card; both rating events are recorded as reviews. Acceptable for single-user.

### File watching

The `notify` crate (`RecommendedWatcher`) watches the cards directory recursively. Events are filtered to `.md` files (ignoring `hashcards.db`, WAL/SHM sidecars, editor swap files, hidden files), debounced over a 250–500 ms window, and coalesced into a single rebuild signal sent through a Tokio `mpsc` channel.

Rebuild semantics:

- Acquire write lock on `cards`.
- Re-parse all `.md` files in the directory.
- Call `db.insert_card(hash, now)` for any genuinely new hashes.
- Drop the lock.
- In-flight read handlers complete on the previous index — acceptable.

Orphans (cards whose hash no longer appears in any file) are **never auto-deleted** by the watcher. Reasons:

1. File edits are reversible (e.g. `git revert`); auto-delete would make orphan-ing irreversible and destroy scheduling history.
2. The existing `hashcards orphans list` / `delete` commands are the deliberate human-in-the-loop checkpoint for cleanup.
3. Iterative editor saves trigger many watcher events; auto-delete would silently destroy history during normal editing flow.

### Watcher fallbacks

- `--rescan-interval <duration>`: in addition to (or instead of) the watcher, a Tokio interval triggers a rebuild. Required for filesystems where inotify is silent — NFS-backed Docker volumes, some macOS Docker Desktop setups, Windows host bind mounts.
- `--no-watch`: disables the watcher entirely. Combine with `--rescan-interval` for poll-only mode.

If `notify` setup fails (unsupported filesystem), the server logs clearly, falls back to interval-based rescan if `--rescan-interval` is set, otherwise aborts startup with a message pointing at the fallback flags.

The watcher only ever rebuilds the parsed-card index. It never deletes cards or rewrites scheduling state.

## HTTP API

| Method | Path | Purpose |
|---|---|---|
| GET | `/` | Dashboard |
| GET | `/drill` | Next due card; supports `?deck=<name>` filter |
| POST | `/rate` | Submit `{hash, rating}`; returns next-card payload as JSON |
| GET | `/file/*path` | Media serving (existing, path-validated) |
| GET | `/static/*` | CSS/JS/KaTeX assets |
| GET | `/favicon.ico` | Existing favicon |
| GET | `/healthz` | `200 OK` once startup completes; for Docker health checks |
| GET | `/api/decks` | JSON: `{deck_name: {due_count, total_count}}` |
| GET | `/api/cards` | JSON: `[{hash, deck, source_file}]`; metadata only, no card text |

The `/api/*` endpoints are read-only metadata (no card text, no review state). They exist so a future LLM-authoring tool can introspect what already exists. They are not used by the dashboard or drill UI.

### Drill flow

1. `GET /drill` → server picks next due card (filtered by `?deck` if set), renders the card front via the existing template path.
2. JS reveals the back when "Show answer" is clicked (existing UX).
3. User clicks a rating button → JS sends `POST /rate` with `{hash, rating}`.
4. Server records review in one DB transaction, computes next card, returns JSON containing the rendered HTML for the next card.
5. JS swaps in the new card. No full page reload between cards.

## Dashboard

### Sections, top to bottom

1. **Status block.** "**N cards due**" + primary "**Drill**" button. Or "You're caught up." when nothing due.
2. **Streak.** Current consecutive-day streak ending today (or yesterday — boundary decided during impl).
3. **Heatmap.** GitHub-contributions-style grid: 53 weeks × 7 days, cells colored by review count (5 buckets: 0, 1–5, 6–15, 16–30, 30+). Hover/tap shows date + count. Server-rendered SVG so it works without JS.
4. **Per-deck breakdown.** Table: deck name, due count, total count. Each name links to `/drill?deck=<name>`. Decks with zero due are dimmed but listed.
5. **Stats grid.** Six tiles: total cards, total reviews, reviewed today, retention (last 30 days), card distribution by maturity (`New / Learning / Young / Mature`).
6. **Footer.** Last drilled timestamp + link to detailed stats page (`stats --format html` output).

### Layout

CSS grid with the existing `768px` breakpoint:

- **Desktop (≥769px):** two-column top — status+streak left, stats grid right. Heatmap full-width. Per-deck table full-width below.
- **Mobile (<768px):** single column, everything stacked. Heatmap horizontally scrollable (53 weeks is too wide for a phone otherwise).

No CSS framework added. New selectors reuse existing color variables and typography. Dark mode follows the existing `prefers-color-scheme` rules.

### New DB query helpers (in `db.rs`)

- `reviews_per_day(start_date, end_date) -> Vec<(Date, u32)>`
- `current_streak(today) -> u32`
- `card_state_distribution() -> {new, learning, young, mature}`
- `retention_rate(window_days) -> f32`
- `total_reviews() -> u64`

## Configuration

All settings exposed as both CLI flags and environment variables (clap `env` attribute). Env vars matter for Docker users running via compose.

| Flag | Env var | Default | Notes |
|---|---|---|---|
| `directory` (positional) | `HASHCARDS_DIRECTORY` | `.` | Cards + DB location |
| `--host` | `HASHCARDS_HOST` | `0.0.0.0` | Container-friendly default (was `127.0.0.1` in drill) |
| `--port` | `HASHCARDS_PORT` | `8000` | |
| `--rescan-interval` | `HASHCARDS_RESCAN_INTERVAL` | unset | Polling fallback (e.g. `30s`) |
| `--no-watch` | `HASHCARDS_NO_WATCH` | `false` | Disable inotify watcher |
| `--from-deck` | `HASHCARDS_FROM_DECK` | unset | Filter |
| `--card-limit` | `HASHCARDS_CARD_LIMIT` | unset | |
| `--new-card-limit` | `HASHCARDS_NEW_CARD_LIMIT` | unset | |
| `--bury-siblings` | `HASHCARDS_BURY_SIBLINGS` | `true` | |
| `--answer-controls` | `HASHCARDS_ANSWER_CONTROLS` | `Full` | |

The `--open-browser` flag from `drill` is not carried over.

## Docker deployment

### Dockerfile (multi-stage, repo root)

- **Builder stage:** `rust:<pinned>-slim`. Cached deps layer, then build `--release`.
- **Runtime stage:** `debian:stable-slim`. Copies the binary; installs `ca-certificates`, `wget` (~1MB; for healthcheck), and `libssl3` if any dep needs it (verified during impl). `EXPOSE 8000`. `WORKDIR /cards`. `ENTRYPOINT ["hashcards"]`. `CMD ["serve", "/cards"]`.
- No `HEALTHCHECK` line in the Dockerfile; documented for `docker-compose.yml` instead.

Distroless was considered but rejected for ergonomics (easier `docker exec` debugging on debian-slim; size difference is small).

### docker-compose example (in `examples/docker/`)

```yaml
services:
  hashcards:
    image: hashcards:latest
    container_name: hashcards
    ports:
      - "8000:8000"
    volumes:
      - /path/to/your/cards:/cards
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "wget -qO- http://localhost:8000/healthz || exit 1"]
      interval: 30s
      timeout: 5s
      retries: 3
```

### Container lifecycle

- SIGTERM → graceful shutdown via Tokio's signal handlers. HTTP server stops accepting new requests, drains in-flight ones, closes the SQLite connection.
- Watcher task dropped during shutdown.
- DB at `/cards/hashcards.db`, lives in the mounted volume, survives restarts.

### Host filesystem and platform notes

- **Linux host + bind mount:** inotify works directly. Recommended.
- **Windows host with WSL2 backend, cards in WSL filesystem:** inotify works directly. Recommended Windows setup.
- **Windows host with cards on Windows filesystem:** inotify often does not propagate across the WSL2 boundary. Use `--rescan-interval 30s`.
- **macOS host:** generally works on modern Docker Desktop with virtiofs; if events are missed, use `--rescan-interval`.
- **NFS / SMB-backed mounts:** inotify does not work. Use `--rescan-interval`.

The README will include a "Self-hosting with Docker" section covering these cases plus backup advice (back up the cards directory; the DB is in there).

## Documentation deliverables

- New "Self-hosting with Docker" section in `README.md`.
- Existing CLI tutorial updated to teach `serve` instead of `drill`.
- `CLAUDE.md` updated: remove the cache rule, add the per-rating persistence rule.
- `CHANGELOG.xml` entry under `<unreleased>`.

## Testing

Following existing patterns (`src/helper.rs`, `:memory:` SQLite for DB tests, `tempfile::TempDir` for filesystem tests).

### DB query unit tests

- `reviews_per_day`: empty, sparse, dense, range edges.
- `current_streak`: zero, one day, multi-day continuous, broken-by-gap, today-vs-yesterday boundary.
- `card_state_distribution`: empty, mixed buckets.
- `retention_rate`: zero reviews edge case (defined behavior), all-good, all-again, mixed.
- `total_reviews`: trivial.

### Per-rating persistence tests

The most important regression suite for the new model:

- Rating a card writes a review record immediately (no buffering).
- Rating updates card performance immediately.
- Server restart between rate calls preserves all ratings.
- Two simultaneous rate calls on different cards both persist.
- Rate call rolls back DB transaction on FSRS computation error.

### HTTP route tests

Using the existing test client pattern against the router:

- `GET /` returns 200, contains expected dashboard sections, reflects DB state.
- `GET /drill` returns next due card; `?deck=X` filter honored; "caught up" state when nothing due.
- `POST /rate` accepts valid input, returns next-card payload, rejects invalid hash with 400.
- `GET /api/decks`, `GET /api/cards` return correct JSON shape and content.
- `GET /healthz` returns 200 once startup completes.
- `GET /file/*` directory traversal rejection (regression for existing behavior).

### File-watcher tests

Real filesystem with `tempfile::TempDir`:

- Adding a `.md` file triggers rebuild; new cards appear in `/api/cards`.
- Editing a card produces a new hash; old hash becomes orphan but is not deleted.
- Removing a `.md` file removes its cards from the index but not from the DB.
- Editor "atomic write" patterns (temp + rename) trigger exactly one rebuild after debouncing.
- `--no-watch` mode does not rebuild on file changes.
- `--rescan-interval` mode rebuilds on the timer even without notify events.

### End-to-end smoke test

One test that:

- Starts the server against a tempdir.
- Drives 5 cards through GET → rate → GET → rate via the HTTP client.
- Adds a new `.md` file mid-run, verifies it appears in the next dashboard load.
- Asserts DB state matches expectations.

### Manual UI verification

Per CLAUDE.md "test the golden path and edge cases":

- Screenshots at 360px / 768px / 1280px viewports.
- Dashboard with cards due, dashboard caught up, dashboard with multiple decks.
- Drill view, drill view caught up.
- Heatmap rendering with sample data.
- Dark mode at each viewport.

### Out of scope for testing

- The `notify` crate itself (trust upstream).
- FSRS algorithm internals (existing `fsrs.rs` tests cover this).
- Markdown rendering (existing tests).

Coverage stays at parity with existing thresholds in `codecov.yml`.

## Open questions

None at design close. Boundary details (debounce window precise value, today-vs-yesterday streak semantics, whether `libssl3` is needed in runtime image) are decided during implementation rather than design.
