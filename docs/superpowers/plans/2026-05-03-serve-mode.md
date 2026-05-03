# `serve` mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the ephemeral `drill` CLI command with a long-running `serve` web app that persists ratings per-card, picks up file edits via inotify, and serves a dashboard with streak, heatmap, and per-deck stats — deployed as a Docker container.

**Architecture:** `cmd/drill/` is renamed to `cmd/serve/` and refactored. Removes the in-memory cache layer in favor of per-rating SQLite writes. Adds a `notify`-backed file watcher rebuilding an `RwLock`-protected card index. New routes: dashboard (`/`), drill view (`/drill?deck=...`), JSON metadata (`/api/decks`, `/api/cards`), health check (`/healthz`). One session row created per server startup; all reviews link to it (no schema change required).

**Tech Stack:** Rust 2024, Axum 0.8, rusqlite 0.39 (bundled), tokio 1.51 (rt-multi-thread + signal + fs), maud 0.27, clap 4.6 with `env` feature, `notify` crate (NEW), Docker with `debian:stable-slim` runtime.

---

## Reference: file map

**Created:**
- `src/cmd/serve/watcher.rs` — file watcher
- `src/cmd/serve/dashboard.rs` — `GET /` handler + helper queries
- `src/cmd/serve/api.rs` — `/api/decks`, `/api/cards`, `/healthz`
- `Dockerfile`
- `examples/docker/docker-compose.yml`

**Renamed:**
- `src/cmd/drill/` → `src/cmd/serve/` (all files)

**Modified:**
- `src/cli.rs` — replace `Drill` subcommand with `Serve`
- `src/main.rs` — module list (drill → serve)
- `src/cmd/mod.rs` — re-export change
- `src/db.rs` — add new query helpers
- `src/types/performance.rs` — add `Maturity` enum + classifier
- `src/cmd/serve/server.rs` — restructure for perpetual + per-rating + watcher
- `src/cmd/serve/state.rs` — `AppState` + `CardIndex`; remove `Cache`, `MutableState`
- `src/cmd/serve/post.rs` — per-rating persistence; remove session-end paths
- `src/cmd/serve/get.rs` — drill view: caught-up state, deck filter
- `src/cmd/serve/template.rs` — base layout reused by dashboard
- `src/cmd/serve/style.css` — dashboard layout, mobile/desktop breakpoints
- `src/cmd/serve/script.js` — minor: optionally support partial card swap (kept compatible)
- `src/cmd/stats.rs` — implement HTML output using shared fragments
- `src/cmd/check.rs` (only if needed for compilation after collection refactor)
- `src/collection.rs` — expose a way to re-parse without re-opening DB (needed by watcher)
- `Cargo.toml` — add `notify` dep
- `README.md` — Self-hosting with Docker section, replace `drill` references with `serve`
- `CLAUDE.md` — remove cache rule, add per-rating rule
- `CHANGELOG.xml` — entry under `<unreleased>`

**Deleted:**
- `src/cmd/serve/cache.rs` (was `src/cmd/drill/cache.rs`)

---

## Task 0: Verify clean baseline

**Files:** none

- [ ] **Step 1: Confirm working tree is clean**

Run: `git status`
Expected: `nothing to commit, working tree clean`

- [ ] **Step 2: Run existing tests to confirm green baseline**

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 3: Verify no commits in progress, on master**

Run: `git rev-parse --abbrev-ref HEAD`
Expected: `master`

---

## Task 1: Add `total_reviews()` DB helper

**Files:**
- Modify: `src/db.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `src/db.rs` (just before the closing `}` of `mod tests`):

```rust
#[test]
fn test_total_reviews() -> Fallible<()> {
    let mut db = Database::new(":memory:")?;
    let card_hash = CardHash::hash_bytes(b"a");
    let now = Timestamp::now();
    db.insert_card(card_hash, now)?;
    assert_eq!(db.total_reviews()?, 0);

    let review = ReviewRecord {
        card_hash,
        reviewed_at: now,
        grade: Grade::Good,
        stability: 2.0,
        difficulty: 2.0,
        interval_raw: 1.0,
        interval_days: 1,
        due_date: now.date(),
    };
    db.save_session(now, now, vec![review])?;
    assert_eq!(db.total_reviews()?, 1);
    Ok(())
}
```

- [ ] **Step 2: Run test to confirm it fails**

Run: `cargo test --lib db::tests::test_total_reviews 2>&1 | tail -20`
Expected: compile error `no method named 'total_reviews' found for struct 'Database'`

- [ ] **Step 3: Implement `total_reviews`**

In `src/db.rs`, add this method to `impl Database` (near `count_reviews_in_date`):

```rust
/// Total number of review records in the database.
pub fn total_reviews(&self) -> Fallible<u64> {
    let sql = "select count(*) from reviews;";
    let count: i64 = self.conn.query_row(sql, [], |row| row.get(0))?;
    Ok(count as u64)
}
```

- [ ] **Step 4: Run test to verify pass**

Run: `cargo test --lib db::tests::test_total_reviews 2>&1 | tail -10`
Expected: `test result: ok. 1 passed`

- [ ] **Step 5: Commit**

```bash
git add src/db.rs
git commit -m "$(cat <<'EOF'
Add Database::total_reviews helper

Used by the upcoming serve dashboard's stats grid.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add `last_review_timestamp()` DB helper

**Files:**
- Modify: `src/db.rs`

- [ ] **Step 1: Write the failing test**

Append inside `mod tests`:

```rust
#[test]
fn test_last_review_timestamp() -> Fallible<()> {
    let mut db = Database::new(":memory:")?;
    assert_eq!(db.last_review_timestamp()?, None);

    let card_hash = CardHash::hash_bytes(b"a");
    let now = Timestamp::now();
    db.insert_card(card_hash, now)?;
    let review = ReviewRecord {
        card_hash,
        reviewed_at: now,
        grade: Grade::Good,
        stability: 2.0,
        difficulty: 2.0,
        interval_raw: 1.0,
        interval_days: 1,
        due_date: now.date(),
    };
    db.save_session(now, now, vec![review])?;
    assert_eq!(db.last_review_timestamp()?, Some(now));
    Ok(())
}
```

- [ ] **Step 2: Run test to confirm failure**

Run: `cargo test --lib db::tests::test_last_review_timestamp 2>&1 | tail -10`
Expected: compile error `no method named 'last_review_timestamp'`

- [ ] **Step 3: Implement**

Add to `impl Database`:

```rust
/// Most recent review timestamp across the entire DB. None if no reviews yet.
pub fn last_review_timestamp(&self) -> Fallible<Option<Timestamp>> {
    let sql = "select max(reviewed_at) from reviews;";
    let result: Option<Timestamp> = self.conn.query_row(sql, [], |row| row.get(0))?;
    Ok(result)
}
```

- [ ] **Step 4: Run test**

Run: `cargo test --lib db::tests::test_last_review_timestamp 2>&1 | tail -10`
Expected: pass

- [ ] **Step 5: Commit**

```bash
git add src/db.rs
git commit -m "$(cat <<'EOF'
Add Database::last_review_timestamp helper

Used by the upcoming serve dashboard footer to show "last drilled".

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add `count_reviews_in_date_range()` DB helper

**Files:**
- Modify: `src/db.rs`

- [ ] **Step 1: Write the failing test**

Append inside `mod tests`:

```rust
#[test]
fn test_count_reviews_in_date_range() -> Fallible<()> {
    use chrono::NaiveDate;
    let mut db = Database::new(":memory:")?;
    let card_hash = CardHash::hash_bytes(b"a");
    let now = Timestamp::now();
    db.insert_card(card_hash, now)?;
    let review = ReviewRecord {
        card_hash,
        reviewed_at: now,
        grade: Grade::Good,
        stability: 2.0,
        difficulty: 2.0,
        interval_raw: 1.0,
        interval_days: 1,
        due_date: now.date(),
    };
    db.save_session(now, now, vec![review])?;

    let today = now.date();
    let yesterday = Date::new(today.into_inner().pred_opt().unwrap());
    let tomorrow = Date::new(today.into_inner().succ_opt().unwrap());

    let counts = db.count_reviews_in_date_range(yesterday, tomorrow)?;
    let on_today: u32 = counts.iter().find(|(d, _)| *d == today).map(|(_, c)| *c).unwrap_or(0);
    assert_eq!(on_today, 1);
    let on_yesterday: u32 = counts.iter().find(|(d, _)| *d == yesterday).map(|(_, c)| *c).unwrap_or(0);
    assert_eq!(on_yesterday, 0);
    Ok(())
}
```

- [ ] **Step 2: Run test, confirm failure**

Run: `cargo test --lib db::tests::test_count_reviews_in_date_range 2>&1 | tail -10`
Expected: compile error.

- [ ] **Step 3: Implement**

Add to `impl Database`:

```rust
/// Count of reviews per day for each date in [start, end] (inclusive).
/// Only days with at least one review are returned.
pub fn count_reviews_in_date_range(
    &self,
    start: Date,
    end: Date,
) -> Fallible<Vec<(Date, u32)>> {
    let sql = "select substr(reviewed_at, 1, 10) as d, count(*) \
               from reviews \
               where d >= ? and d <= ? \
               group by d \
               order by d;";
    let mut stmt = self.conn.prepare(sql)?;
    let rows = stmt.query_map(params![start, end], |row| {
        let d: Date = row.get(0)?;
        let c: i64 = row.get(1)?;
        Ok((d, c as u32))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}
```

- [ ] **Step 4: Run test**

Run: `cargo test --lib db::tests::test_count_reviews_in_date_range 2>&1 | tail -10`
Expected: pass

- [ ] **Step 5: Commit**

```bash
git add src/db.rs
git commit -m "$(cat <<'EOF'
Add Database::count_reviews_in_date_range helper

Returns per-day review counts in a date range; used to render the
serve dashboard's heatmap.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Add `current_streak()` DB helper

**Files:**
- Modify: `src/db.rs`

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests`:

```rust
#[test]
fn test_current_streak_zero() -> Fallible<()> {
    let db = Database::new(":memory:")?;
    let today = Date::today();
    assert_eq!(db.current_streak(today)?, 0);
    Ok(())
}

#[test]
fn test_current_streak_today_only() -> Fallible<()> {
    let mut db = Database::new(":memory:")?;
    let now = Timestamp::now();
    let card_hash = CardHash::hash_bytes(b"a");
    db.insert_card(card_hash, now)?;
    let review = ReviewRecord {
        card_hash, reviewed_at: now, grade: Grade::Good,
        stability: 2.0, difficulty: 2.0, interval_raw: 1.0, interval_days: 1,
        due_date: now.date(),
    };
    db.save_session(now, now, vec![review])?;
    assert_eq!(db.current_streak(now.date())?, 1);
    Ok(())
}

#[test]
fn test_current_streak_continuous() -> Fallible<()> {
    use chrono::Duration;
    let mut db = Database::new(":memory:")?;
    let today = Date::today();
    let card_hash = CardHash::hash_bytes(b"a");
    let now = Timestamp::now();
    db.insert_card(card_hash, now)?;

    // Three reviews on three consecutive days ending today.
    for offset in [-2, -1, 0] {
        let d = today.into_inner() + Duration::days(offset);
        let ts = Timestamp::new(d.and_hms_opt(12, 0, 0).unwrap());
        let review = ReviewRecord {
            card_hash, reviewed_at: ts, grade: Grade::Good,
            stability: 2.0, difficulty: 2.0, interval_raw: 1.0, interval_days: 1,
            due_date: ts.date(),
        };
        db.save_session(ts, ts, vec![review])?;
    }
    assert_eq!(db.current_streak(today)?, 3);
    Ok(())
}

#[test]
fn test_current_streak_broken_by_gap() -> Fallible<()> {
    use chrono::Duration;
    let mut db = Database::new(":memory:")?;
    let today = Date::today();
    let card_hash = CardHash::hash_bytes(b"a");
    let now = Timestamp::now();
    db.insert_card(card_hash, now)?;

    // Today and 3 days ago — gap means streak is just today.
    for offset in [-3i64, 0i64] {
        let d = today.into_inner() + Duration::days(offset);
        let ts = Timestamp::new(d.and_hms_opt(12, 0, 0).unwrap());
        let review = ReviewRecord {
            card_hash, reviewed_at: ts, grade: Grade::Good,
            stability: 2.0, difficulty: 2.0, interval_raw: 1.0, interval_days: 1,
            due_date: ts.date(),
        };
        db.save_session(ts, ts, vec![review])?;
    }
    assert_eq!(db.current_streak(today)?, 1);
    Ok(())
}

#[test]
fn test_current_streak_yesterday_counts() -> Fallible<()> {
    use chrono::Duration;
    // If today has no reviews but yesterday does, streak counts back from yesterday.
    let mut db = Database::new(":memory:")?;
    let today = Date::today();
    let card_hash = CardHash::hash_bytes(b"a");
    let now = Timestamp::now();
    db.insert_card(card_hash, now)?;
    let yesterday = today.into_inner() + Duration::days(-1);
    let ts = Timestamp::new(yesterday.and_hms_opt(12, 0, 0).unwrap());
    let review = ReviewRecord {
        card_hash, reviewed_at: ts, grade: Grade::Good,
        stability: 2.0, difficulty: 2.0, interval_raw: 1.0, interval_days: 1,
        due_date: ts.date(),
    };
    db.save_session(ts, ts, vec![review])?;
    assert_eq!(db.current_streak(today)?, 1);
    Ok(())
}
```

- [ ] **Step 2: Run, confirm failures**

Run: `cargo test --lib db::tests::test_current_streak 2>&1 | tail -20`
Expected: compile error `no method named 'current_streak'`

- [ ] **Step 3: Implement**

Add to `impl Database`:

```rust
/// Current consecutive-day streak of review activity ending at `today`.
/// If `today` has no reviews but `today - 1` does, the streak counts back
/// from yesterday — this avoids a fresh streak resetting before the user
/// has done their reviews for the day.
pub fn current_streak(&self, today: Date) -> Fallible<u32> {
    use chrono::Duration;
    use std::collections::HashSet;

    let sql = "select distinct substr(reviewed_at, 1, 10) from reviews;";
    let mut stmt = self.conn.prepare(sql)?;
    let mut active: HashSet<Date> = HashSet::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let d: Date = row.get(0)?;
        active.insert(d);
    }

    if active.is_empty() {
        return Ok(0);
    }

    // Anchor: today if today has reviews, else yesterday if yesterday has reviews, else 0.
    let today_in = today.into_inner();
    let yesterday = Date::new(today_in + Duration::days(-1));
    let mut cursor: Date = if active.contains(&today) {
        today
    } else if active.contains(&yesterday) {
        yesterday
    } else {
        return Ok(0);
    };

    let mut streak: u32 = 0;
    loop {
        if active.contains(&cursor) {
            streak += 1;
            cursor = Date::new(cursor.into_inner() + Duration::days(-1));
        } else {
            break;
        }
    }
    Ok(streak)
}
```

- [ ] **Step 4: Run all streak tests**

Run: `cargo test --lib db::tests::test_current_streak 2>&1 | tail -15`
Expected: 5 passed

- [ ] **Step 5: Commit**

```bash
git add src/db.rs
git commit -m "$(cat <<'EOF'
Add Database::current_streak helper

Returns consecutive-day review streak ending at the given date,
counting from yesterday if today has no reviews yet (to avoid
resetting the user's streak before they've done their daily review).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Add `retention_rate(window_days)` DB helper

**Files:**
- Modify: `src/db.rs`

Retention = `Good`/`Easy` ratings as fraction of all ratings in window. Reviews where the user "passed" the card.

- [ ] **Step 1: Write the failing test**

Append inside `mod tests`:

```rust
#[test]
fn test_retention_rate() -> Fallible<()> {
    let mut db = Database::new(":memory:")?;
    let now = Timestamp::now();
    let card_hash = CardHash::hash_bytes(b"a");
    db.insert_card(card_hash, now)?;

    // No reviews => 0.0 by convention.
    assert_eq!(db.retention_rate(30)?, 0.0);

    // 3 Good, 1 Forgot in window: retention = 0.75
    let make_review = |grade: Grade| ReviewRecord {
        card_hash, reviewed_at: now, grade,
        stability: 2.0, difficulty: 2.0, interval_raw: 1.0, interval_days: 1,
        due_date: now.date(),
    };
    db.save_session(now, now, vec![
        make_review(Grade::Good),
        make_review(Grade::Good),
        make_review(Grade::Good),
        make_review(Grade::Forgot),
    ])?;
    let r = db.retention_rate(30)?;
    assert!((r - 0.75).abs() < 1e-6, "expected ~0.75, got {r}");
    Ok(())
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo test --lib db::tests::test_retention_rate 2>&1 | tail -10`
Expected: compile error.

- [ ] **Step 3: Implement**

Add to `impl Database`:

```rust
/// Fraction of ratings in the last `window_days` that were Good or Easy.
/// Returns 0.0 if there are no reviews in the window.
pub fn retention_rate(&self, window_days: i64) -> Fallible<f32> {
    use chrono::Duration;
    let cutoff = Date::today().into_inner() + Duration::days(-window_days);
    let cutoff = Date::new(cutoff);
    let sql = "select grade from reviews where substr(reviewed_at, 1, 10) >= ?;";
    let mut stmt = self.conn.prepare(sql)?;
    let mut total: u32 = 0;
    let mut passed: u32 = 0;
    let mut rows = stmt.query(params![cutoff])?;
    while let Some(row) = rows.next()? {
        let grade: Grade = row.get(0)?;
        total += 1;
        if matches!(grade, Grade::Good | Grade::Easy) {
            passed += 1;
        }
    }
    if total == 0 {
        Ok(0.0)
    } else {
        Ok(passed as f32 / total as f32)
    }
}
```

- [ ] **Step 4: Run test**

Run: `cargo test --lib db::tests::test_retention_rate 2>&1 | tail -10`
Expected: pass

- [ ] **Step 5: Commit**

```bash
git add src/db.rs
git commit -m "$(cat <<'EOF'
Add Database::retention_rate helper

Fraction of Good/Easy ratings over a rolling window; powers the
retention tile on the serve dashboard.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Add card maturity classification

**Files:**
- Modify: `src/types/performance.rs`

Maturity buckets: `New` (no reviews), `Learning` (interval < 7 days), `Young` (7 ≤ interval < 30), `Mature` (≥ 30 days). These thresholds match common SRS conventions.

- [ ] **Step 1: Write the failing test**

Append at the bottom of `src/types/performance.rs` (before any existing `#[cfg(test)]` block, or add one):

```rust
#[cfg(test)]
mod maturity_tests {
    use super::*;

    fn perf(interval_days: i64) -> Performance {
        Performance::Reviewed(ReviewedPerformance {
            last_reviewed_at: Timestamp::now(),
            stability: 1.0,
            difficulty: 1.0,
            interval_raw: interval_days as f64,
            interval_days,
            due_date: Date::today(),
            review_count: 1,
        })
    }

    #[test]
    fn test_maturity_new() {
        assert_eq!(Performance::New.maturity(), Maturity::New);
    }

    #[test]
    fn test_maturity_learning() {
        assert_eq!(perf(0).maturity(), Maturity::Learning);
        assert_eq!(perf(6).maturity(), Maturity::Learning);
    }

    #[test]
    fn test_maturity_young() {
        assert_eq!(perf(7).maturity(), Maturity::Young);
        assert_eq!(perf(29).maturity(), Maturity::Young);
    }

    #[test]
    fn test_maturity_mature() {
        assert_eq!(perf(30).maturity(), Maturity::Mature);
        assert_eq!(perf(365).maturity(), Maturity::Mature);
    }
}
```

You may need to add `use crate::types::date::Date;` and `use crate::types::timestamp::Timestamp;` inside that test mod's scope if not already imported via `super::*`.

- [ ] **Step 2: Run, confirm failure**

Run: `cargo test --lib types::performance::maturity_tests 2>&1 | tail -10`
Expected: compile error `cannot find type 'Maturity'`

- [ ] **Step 3: Implement**

Append to `src/types/performance.rs` (after the `update_performance` function):

```rust
/// Card maturity bucket for stats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Maturity {
    New,
    Learning,
    Young,
    Mature,
}

impl Maturity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Maturity::New => "New",
            Maturity::Learning => "Learning",
            Maturity::Young => "Young",
            Maturity::Mature => "Mature",
        }
    }
}

impl Performance {
    pub fn maturity(&self) -> Maturity {
        match self {
            Performance::New => Maturity::New,
            Performance::Reviewed(rp) => {
                if rp.interval_days < 7 {
                    Maturity::Learning
                } else if rp.interval_days < 30 {
                    Maturity::Young
                } else {
                    Maturity::Mature
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib types::performance::maturity_tests 2>&1 | tail -10`
Expected: 4 passed

- [ ] **Step 5: Commit**

```bash
git add src/types/performance.rs
git commit -m "$(cat <<'EOF'
Add Maturity enum and Performance::maturity classifier

Buckets cards by interval: <7d Learning, 7-29d Young, >=30d Mature,
unreviewed New. Used by the serve dashboard stats grid.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Rename `cmd/drill/` to `cmd/serve/`

This is a mechanical rename to set up the next refactors. We do *not* yet remove the cache or change persistence — only the directory name and references. The behavior remains identical at this step.

**Files:**
- Renamed: `src/cmd/drill/` → `src/cmd/serve/` (entire directory)
- Modify: `src/cmd/mod.rs`
- Modify: `src/cli.rs` (import paths only at this step)

- [ ] **Step 1: Inspect `src/cmd/mod.rs` to see current declaration**

Run: `cat src/cmd/mod.rs`
Expected output includes a line like `pub mod drill;`

- [ ] **Step 2: Rename the directory**

Run: `git mv src/cmd/drill src/cmd/serve`
Expected: silent, no error.

- [ ] **Step 3: Update `src/cmd/mod.rs`**

Edit `src/cmd/mod.rs` and change `pub mod drill;` to `pub mod serve;`. (And any similar `mod drill;` lines.)

- [ ] **Step 4: Update import paths globally**

Run:
```bash
grep -rln "crate::cmd::drill" src/
```

For every file listed, replace `crate::cmd::drill` with `crate::cmd::serve` using `sed`:

```bash
grep -rln "crate::cmd::drill" src/ | xargs sed -i 's|crate::cmd::drill|crate::cmd::serve|g'
```

- [ ] **Step 5: Verify build**

Run: `cargo build 2>&1 | tail -20`
Expected: build succeeds. (Tests not yet checked because some test names in `cmd/serve/mod.rs` reference `cmd::drill` patterns; those files were renamed too so should be fine.)

- [ ] **Step 6: Run tests**

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Rename src/cmd/drill to src/cmd/serve

Mechanical rename in preparation for the serve mode rewrite.
No behavior changes; the existing `drill` subcommand still works
because cli.rs imports were updated to point at the new path.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Replace `Drill` subcommand with `Serve` in CLI

This adds the `Serve` subcommand and removes `Drill`. The behavior change at this step is minimal: same flags (minus `--open-browser`), `--host` defaults to `0.0.0.0`, env var support added, no auto-browser. Persistence model still unchanged (per-rating refactor comes later).

**Files:**
- Modify: `src/cli.rs`

- [ ] **Step 1: Replace the Drill arm of the Command enum**

Open `src/cli.rs`. Replace the entire `Drill { ... }` variant with:

```rust
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
```

- [ ] **Step 2: Replace the match arm in `entrypoint`**

In `pub async fn entrypoint()`, replace the entire `Command::Drill { ... } => { ... }` arm with:

```rust
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
```

- [ ] **Step 3: Remove unused imports**

In `src/cli.rs`, remove these imports (no longer used):

```rust
use std::process::exit;
use tokio::spawn;
use crate::utils::wait_for_server;
```

- [ ] **Step 4: Add new fields to `ServerConfig` so it compiles**

Open `src/cmd/serve/server.rs` and locate `pub struct ServerConfig`. Add two fields:

```rust
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
    pub rescan_interval: Option<String>,  // NEW
    pub no_watch: bool,                    // NEW
}
```

These fields are unused at this step (no watcher yet); they wire up cleanly in Task 14.

- [ ] **Step 5: Update existing test fixtures**

Open `src/cmd/serve/mod.rs`. In every `let config = ServerConfig { ... }` block in the tests, add at the end:

```rust
            rescan_interval: None,
            no_watch: false,
```

- [ ] **Step 6: Build and test**

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: all pass.

- [ ] **Step 7: Verify CLI exposes Serve, not Drill**

Run: `cargo run -- --help 2>&1 | tail -30`
Expected: lists `serve` and not `drill`.

Run: `cargo run -- serve --help 2>&1 | tail -40`
Expected: lists `--host`, `--port`, `--rescan-interval`, `--no-watch`, etc., with `--host` showing default `0.0.0.0`.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Replace drill subcommand with serve

Adds the serve subcommand alongside removing drill. New defaults:
--host=0.0.0.0 (container-friendly), env var support on every flag.
Adds --rescan-interval and --no-watch flags (wired in Task 14).
Drops --open-browser (no auto-browser in serve mode).

Persistence model is still session-based at this step; per-rating
persistence and the watcher land in subsequent tasks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Strip session-end and Shutdown semantics from serve

`serve` runs forever. It does not "complete" a session. The shutdown button, the completion page, the `finished_at` field, and the `Action::Shutdown` variant are all removed.

**Files:**
- Modify: `src/cmd/serve/server.rs`
- Modify: `src/cmd/serve/state.rs`
- Modify: `src/cmd/serve/post.rs`
- Modify: `src/cmd/serve/get.rs`

- [ ] **Step 1: Remove `Action::Shutdown` and `Action::End` from post.rs**

In `src/cmd/serve/post.rs`, replace the `Action` enum with:

```rust
#[derive(Debug, Deserialize)]
enum Action {
    Reveal,
    Undo,
    Forgot,
    Hard,
    Good,
    Easy,
}
```

In `action_handler`, delete the `Action::End => { finish_session(...) }` and `Action::Shutdown => { ... }` arms.

In the `Action::Forgot | Action::Hard | Action::Good | Action::Easy` arm, delete the lines:

```rust
                // Was this the last card?
                if mutable.cards.is_empty() {
                    finish_session(&mut mutable, &state)?;
                }
```

Delete the entire `fn finish_session(...)` function.

- [ ] **Step 2: Remove `finished_at` from MutableState and shutdown_tx from ServerState**

In `src/cmd/serve/state.rs`, delete the `pub finished_at: Option<Timestamp>` field from `MutableState` and the `pub shutdown_tx: Arc<Mutex<Option<Sender<()>>>>` field from `ServerState`.

Remove these now-unused imports from `state.rs`:

```rust
use tokio::sync::oneshot::Sender;
use crate::types::timestamp::Timestamp;
```

(Re-add `Timestamp` only if other fields still use it.)

- [ ] **Step 3: Update server.rs**

In `src/cmd/serve/server.rs`:

- Delete `use tokio::sync::oneshot::channel;` and `use tokio::sync::oneshot::Receiver;`.
- Delete the `let (shutdown_tx, shutdown_rx) = channel();` line.
- Remove `shutdown_tx` from the `ServerState { ... }` literal.
- Remove `finished_at: None,` from `MutableState { ... }`.
- Replace the post-`axum::serve` block:

  ```rust
      // Check if session was complete when server shut down
      let mutable = state.mutable.lock().unwrap();
      if mutable.finished_at.is_some() {
          Ok(())
      } else {
          fail("Session interrupted before completion")
      }
  ```

  With:

  ```rust
      Ok(())
  ```

- Replace `shutdown_signal(shutdown_rx)` in the graceful_shutdown call with a function that only listens for SIGTERM/Ctrl+C. Replace the `shutdown_signal` function entirely with:

  ```rust
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
  ```

  And update the call: `.with_graceful_shutdown(shutdown_signal())`.

- Remove the `fail` import if no longer used.

- [ ] **Step 4: Strip the completion page from get.rs**

In `src/cmd/serve/get.rs`:

- Delete `fn render_completion_page` entirely.
- In `inner`, replace the body with:

  ```rust
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
  ```

- Add a `render_caught_up` function:

  ```rust
  fn render_caught_up() -> Markup {
      html! {
          div.caught-up {
              h1 { "You're caught up." }
              p { a href="/" { "← Dashboard" } }
          }
      }
  }
  ```

- In `render_session_page`, remove the "End" button rendering. Replace the existing `(end_button())` and the `fn end_button()` definition with nothing — delete both. Also delete the `Shutdown` form rendering in any place it appears.

- Remove the `TS_FORMAT` constant (was used by completion page).

- [ ] **Step 5: Update tests**

Open `src/cmd/serve/mod.rs`. The tests `test_e2e`, `test_end`, and any test asserting `"Session Completed"` or `"action=End"` will fail because End/Shutdown actions no longer exist.

For now, as a transitional measure: delete `test_end` entirely. In `test_e2e`, replace the final two assertions:

```rust
        assert!(html.contains("Session Completed"));
```

with assertions appropriate to the caught-up state. Replace the entire post-`Reveal`/`Good` end-of-session block with:

```rust
        // After rating the final card, we should see the caught-up screen.
        let response = reqwest::Client::new()
            .post(format!("http://{TEST_HOST}:{port}/"))
            .form(&[("action", "Good")])
            .send()
            .await?;
        assert!(response.status().is_success());
        let html = response.text().await?;
        assert!(html.contains("You're caught up."));
```

The behavior is now: after rating all cards, page shows "You're caught up." instead of session summary.

- [ ] **Step 6: Build and test**

Run: `cargo build 2>&1 | tail -10`
Expected: builds.

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Strip session-end semantics from serve mode

Remove the End button, Shutdown action, completion page, and the
finished_at field. Serve runs forever (or until SIGTERM/Ctrl+C) and
shows a "You're caught up." screen when the in-memory queue is empty.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Per-rating persistence (delete cache, write through to DB)

This is the core persistence model change. The in-memory `Cache` is removed; rating handlers write directly to SQLite in a single transaction.

**Files:**
- Delete: `src/cmd/serve/cache.rs`
- Modify: `src/cmd/serve/mod.rs` (remove `mod cache`)
- Modify: `src/cmd/serve/state.rs`
- Modify: `src/cmd/serve/post.rs`
- Modify: `src/cmd/serve/server.rs`
- Modify: `src/db.rs` (add `save_review` helper writing one review to a session)

The new flow per rating:

1. Lock `MutableState` (existing pattern).
2. Pop the front card.
3. Compute new performance via `update_performance`.
4. Open a DB transaction.
5. Insert review row referencing the session created at startup.
6. Update card performance row.
7. Commit transaction.
8. If `should_repeat`, push card to back of queue.

- [ ] **Step 1: Add `save_review` helper to Database with a failing test**

In `src/db.rs`, append inside `mod tests`:

```rust
#[test]
fn test_save_review_appends_to_session() -> Fallible<()> {
    let mut db = Database::new(":memory:")?;
    let now = Timestamp::now();
    let card_hash = CardHash::hash_bytes(b"a");
    db.insert_card(card_hash, now)?;
    let session_id = db.create_session(now)?;
    let review = ReviewRecord {
        card_hash, reviewed_at: now, grade: Grade::Good,
        stability: 2.0, difficulty: 2.0, interval_raw: 1.0, interval_days: 1,
        due_date: now.date(),
    };
    db.save_review(session_id, &review)?;
    let reviews = db.get_reviews_for_session(session_id)?;
    assert_eq!(reviews.len(), 1);
    Ok(())
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo test --lib db::tests::test_save_review_appends_to_session 2>&1 | tail -10`
Expected: compile errors (`create_session`, `save_review` undefined).

- [ ] **Step 3: Implement helpers**

In `src/db.rs`, add to `impl Database`:

```rust
/// Create an empty session and return its ID. ended_at is initialized
/// to started_at; serve mode does not currently update it after creation.
pub fn create_session(&mut self, started_at: Timestamp) -> Fallible<i64> {
    let sql = "insert into sessions (started_at, ended_at) values (?, ?) returning session_id;";
    let session_id: i64 = self.conn.query_row(
        sql,
        params![started_at, started_at],
        |row| row.get(0),
    )?;
    Ok(session_id)
}

/// Append a single review to an existing session.
pub fn save_review(&self, session_id: i64, review: &ReviewRecord) -> Fallible<()> {
    let sql = "insert into reviews (session_id, card_hash, reviewed_at, grade, stability, difficulty, interval_raw, interval_days, due_date) values (?, ?, ?, ?, ?, ?, ?, ?, ?);";
    self.conn.execute(
        sql,
        params![
            session_id,
            review.card_hash,
            review.reviewed_at,
            review.grade,
            review.stability,
            review.difficulty,
            review.interval_raw,
            review.interval_days as i32,
            review.due_date
        ],
    )?;
    Ok(())
}

/// Insert a review and update the card performance in a single transaction.
/// Used by serve mode for per-rating persistence.
pub fn record_rating(
    &mut self,
    session_id: i64,
    review: &ReviewRecord,
    new_performance: Performance,
) -> Fallible<()> {
    let tx = self.conn.transaction()?;
    let sql = "insert into reviews (session_id, card_hash, reviewed_at, grade, stability, difficulty, interval_raw, interval_days, due_date) values (?, ?, ?, ?, ?, ?, ?, ?, ?);";
    tx.execute(
        sql,
        params![
            session_id,
            review.card_hash,
            review.reviewed_at,
            review.grade,
            review.stability,
            review.difficulty,
            review.interval_raw,
            review.interval_days as i32,
            review.due_date
        ],
    )?;
    let (last_reviewed_at, stability, difficulty, interval_raw, interval_days, due_date, review_count) =
        match new_performance {
            Performance::New => (None, None, None, None, None::<i32>, None, 0i32),
            Performance::Reviewed(rp) => (
                Some(rp.last_reviewed_at),
                Some(rp.stability),
                Some(rp.difficulty),
                Some(rp.interval_raw),
                Some(rp.interval_days as i32),
                Some(rp.due_date),
                rp.review_count as i32,
            ),
        };
    let sql = "update cards set last_reviewed_at = ?, stability = ?, difficulty = ?, interval_raw = ?, interval_days = ?, due_date = ?, review_count = ? where card_hash = ?;";
    tx.execute(
        sql,
        params![
            last_reviewed_at,
            stability,
            difficulty,
            interval_raw,
            interval_days,
            due_date,
            review_count,
            review.card_hash
        ],
    )?;
    tx.commit()?;
    Ok(())
}
```

You will need this import at the top of `db.rs`:

```rust
use crate::types::performance::Performance;
```

- [ ] **Step 4: Add a regression test for `record_rating` atomicity**

Append in `mod tests`:

```rust
#[test]
fn test_record_rating_single_transaction() -> Fallible<()> {
    let mut db = Database::new(":memory:")?;
    let now = Timestamp::now();
    let card_hash = CardHash::hash_bytes(b"a");
    db.insert_card(card_hash, now)?;
    let session_id = db.create_session(now)?;
    let review = ReviewRecord {
        card_hash, reviewed_at: now, grade: Grade::Good,
        stability: 2.0, difficulty: 2.0, interval_raw: 1.0, interval_days: 5,
        due_date: now.date(),
    };
    let perf = Performance::Reviewed(ReviewedPerformance {
        last_reviewed_at: now,
        stability: 2.0, difficulty: 2.0, interval_raw: 1.0, interval_days: 5,
        due_date: now.date(), review_count: 1,
    });
    db.record_rating(session_id, &review, perf)?;
    assert_eq!(db.total_reviews()?, 1);
    let fetched = db.get_card_performance(card_hash)?;
    assert_eq!(fetched, perf);
    Ok(())
}
```

- [ ] **Step 5: Run all new tests**

Run: `cargo test --lib db::tests::test_save_review_appends_to_session db::tests::test_record_rating_single_transaction 2>&1 | tail -10`
Expected: 2 passed.

- [ ] **Step 6: Commit DB layer changes alone**

```bash
git add src/db.rs
git commit -m "$(cat <<'EOF'
Add Database helpers for per-rating persistence

Adds create_session, save_review, and record_rating. record_rating
inserts the review and updates the card performance in a single
transaction; serve mode will use it for per-rating writes.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 7: Refactor `state.rs` — remove Cache, add session_id**

Open `src/cmd/serve/state.rs`. Replace its contents with:

```rust
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

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use crate::cmd::serve::server::AnswerControls;
use crate::db::Database;
use crate::db::ReviewRecord;
use crate::fsrs::Difficulty;
use crate::fsrs::Grade;
use crate::fsrs::Stability;
use crate::types::card::Card;
use crate::types::date::Date;
use crate::types::timestamp::Timestamp;

#[derive(Clone)]
pub struct ServerState {
    pub port: u16,
    pub directory: PathBuf,
    pub macros: Vec<(String, String)>,
    pub session_id: i64,
    pub mutable: Arc<Mutex<MutableState>>,
    pub answer_controls: AnswerControls,
}

pub struct MutableState {
    pub reveal: bool,
    pub db: Database,
    pub cards: Vec<Card>,
    pub reviews: Vec<Review>,
}

#[derive(Clone)]
pub struct Review {
    pub card: Card,
    pub reviewed_at: Timestamp,
    pub grade: Grade,
    pub stability: Stability,
    pub difficulty: Difficulty,
    pub interval_raw: f64,
    pub interval_days: i64,
    pub due_date: Date,
}

impl Review {
    pub fn should_repeat(&self) -> bool {
        self.grade == Grade::Forgot || self.grade == Grade::Hard
    }

    pub fn into_record(self) -> ReviewRecord {
        ReviewRecord {
            card_hash: self.card.hash(),
            reviewed_at: self.reviewed_at,
            grade: self.grade,
            stability: self.stability,
            difficulty: self.difficulty,
            interval_raw: self.interval_raw,
            interval_days: self.interval_days,
            due_date: self.due_date,
        }
    }
}
```

(`Cache`-related fields are gone; `total_cards` and `session_started_at` are gone since the dashboard derives those values directly from the DB on each request.)

- [ ] **Step 8: Refactor `post.rs` — write through, drop cache**

Replace `src/cmd/serve/post.rs` with:

```rust
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

use axum::Form;
use axum::extract::State;
use axum::response::Redirect;
use serde::Deserialize;

use crate::cmd::serve::state::Review;
use crate::cmd::serve::state::ServerState;
use crate::error::Fallible;
use crate::fsrs::Grade;
use crate::types::card::Card;
use crate::types::card_hash::CardHash;
use crate::types::performance::Performance;
use crate::types::performance::ReviewedPerformance;
use crate::types::performance::update_performance;
use crate::types::timestamp::Timestamp;

#[derive(Debug, Deserialize)]
enum Action {
    Reveal,
    Undo,
    Forgot,
    Hard,
    Good,
    Easy,
}

impl Action {
    pub fn grade(&self) -> Grade {
        match self {
            Action::Forgot => Grade::Forgot,
            Action::Hard => Grade::Hard,
            Action::Good => Grade::Good,
            Action::Easy => Grade::Easy,
            _ => panic!("Action does not correspond to a grade"),
        }
    }
}

#[derive(Deserialize)]
pub struct FormData {
    action: Action,
}

pub async fn post_handler(
    State(state): State<ServerState>,
    Form(form): Form<FormData>,
) -> Redirect {
    if let Err(e) = action_handler(state, form.action).await {
        log::error!("error: {e}");
    }
    Redirect::to("/drill")
}

async fn action_handler(state: ServerState, action: Action) -> Fallible<()> {
    let mut mutable = state.mutable.lock().unwrap();
    match action {
        Action::Reveal => {
            mutable.reveal = true;
        }
        Action::Undo => {
            // Per-rating undo: best-effort, requires re-reading the previous
            // performance from DB and rolling back the last review row. The
            // current undo behavior of the drill UI relied on the cache; we
            // simplify by making Undo a no-op in serve mode (the request that
            // last rated a card has already been committed to disk). The Undo
            // button is hidden in get.rs.
        }
        Action::Forgot | Action::Hard | Action::Good | Action::Easy => {
            if !mutable.reveal {
                return Ok(());
            }
            if mutable.cards.is_empty() {
                return Ok(());
            }
            let reviewed_at: Timestamp = Timestamp::now();
            let card: Card = mutable.cards.remove(0);
            let hash: CardHash = card.hash();
            let grade: Grade = action.grade();

            // Read current performance from DB (no cache).
            let prior: Performance = mutable.db.get_card_performance(hash)?;
            let new_perf: ReviewedPerformance = update_performance(prior, grade, reviewed_at);
            let new_perf_enum = Performance::Reviewed(new_perf);

            let review = Review {
                card: card.clone(),
                reviewed_at,
                grade,
                stability: new_perf.stability,
                difficulty: new_perf.difficulty,
                interval_raw: new_perf.interval_raw,
                interval_days: new_perf.interval_days,
                due_date: new_perf.due_date,
            };

            // Persist atomically: review + performance update in one transaction.
            let record = review.clone().into_record();
            mutable.db.record_rating(state.session_id, &record, new_perf_enum)?;

            if review.should_repeat() {
                mutable.cards.push(card);
            }
            mutable.reviews.push(review);
            mutable.reveal = false;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_grade() {
        assert_eq!(Action::Forgot.grade(), Grade::Forgot);
        assert_eq!(Action::Hard.grade(), Grade::Hard);
        assert_eq!(Action::Good.grade(), Grade::Good);
        assert_eq!(Action::Easy.grade(), Grade::Easy);
    }
}
```

- [ ] **Step 9: Update `state.rs` — `Review` needs `Clone` derived (already is)** — verify

In `src/cmd/serve/state.rs`, confirm `Review` already has `#[derive(Clone)]`. (It does in the snippet above.)

- [ ] **Step 10: Hide the Undo button in get.rs**

In `src/cmd/serve/get.rs`, replace `(undo_button(undo_disabled))` everywhere with empty space (delete those lines), and delete the `fn undo_button` definition. Remove the `undo_disabled` local. The button is gone in serve mode — undo doesn't fit per-rating semantics.

- [ ] **Step 11: Update server.rs to use the new state shape and create a startup session**

In `src/cmd/serve/server.rs`, replace the body of `start_server` with:

```rust
pub async fn start_server(config: ServerConfig) -> Fallible<()> {
    let Collection {
        directory,
        mut db,
        cards,
        macros,
    } = Collection::new(config.directory)?;

    let today: Date = config.session_started_at.date();

    let db_hashes: HashSet<CardHash> = db.card_hashes()?;
    for card in cards.iter() {
        if !db_hashes.contains(&card.hash()) {
            db.insert_card(card.hash(), config.session_started_at)?;
        }
    }

    let session_id: i64 = db.create_session(config.session_started_at)?;

    // Compute the initial due-set the same way drill did.
    let due_today: HashSet<CardHash> = db.due_today(today)?;
    let due_today: Vec<Card> = cards
        .into_iter()
        .filter(|card| due_today.contains(&card.hash()))
        .collect();

    let due_today: Vec<Card> = filter_deck(
        &db,
        due_today,
        config.card_limit,
        config.new_card_limit,
        config.deck_filter,
    )?;

    let due_today: Vec<Card> = if config.bury_siblings {
        bury_siblings(due_today)
    } else {
        due_today
    };

    let due_today: Vec<Card> = if config.shuffle {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let mut rng = TinyRng::from_seed(seed);
        shuffle(due_today, &mut rng)
    } else {
        due_today
    };

    let state = ServerState {
        port: config.port,
        directory,
        macros,
        session_id,
        mutable: Arc::new(Mutex::new(MutableState {
            reveal: false,
            db,
            cards: due_today,
            reviews: Vec::new(),
        })),
        answer_controls: config.answer_controls,
    };

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
```

(Note: `Collection::new` returns `db` not `mut db`; mark it `mut` in the destructure — `mut db,` — so we can call `create_session` which needs `&mut self`.)

Remove unused imports from server.rs:

```rust
use crate::cmd::serve::cache::Cache;
```

Also remove `total_cards` and `session_started_at` from any remaining `ServerState` construction (handled by replacement above).

- [ ] **Step 12: Delete cache.rs and unhook from mod.rs**

```bash
rm src/cmd/serve/cache.rs
```

In `src/cmd/serve/mod.rs`, delete the line `mod cache;`.

- [ ] **Step 13: Update `get.rs` — remove total_cards / session_started_at references**

In `src/cmd/serve/get.rs`, the function `render_session_page` references `state.total_cards` and `state.session_started_at`. Replace the progress computation:

```rust
fn render_session_page(state: &ServerState, mutable: &MutableState) -> Fallible<Markup> {
    let remaining = mutable.cards.len();
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
```

- [ ] **Step 14: Update test fixtures in `cmd/serve/mod.rs`**

The tests construct `ServerConfig` and may have stopped compiling because of the removed fields and the new shape of state. Run the test suite and fix imports/fields one error at a time.

Run: `cargo build 2>&1 | tail -30`
Expected: should compile after iterations. If errors mention `total_cards`, `session_started_at`, `finished_at` in test code, delete those references.

- [ ] **Step 15: Run full test suite**

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: all pass.

- [ ] **Step 16: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Switch serve to per-rating persistence; remove cache layer

Each rating now writes through to SQLite in a single transaction
(insert review + update card performance). Cache module is deleted.
A startup-time session row is created and used as the parent for
all reviews recorded during the server's lifetime.

The Undo button is removed (its semantics required the cache);
session-end / completion / shutdown UI was already removed in the
prior task.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Recompute due-set per request from DB + CardIndex

The startup-time `Vec<Card>` snapshot is replaced by a live `CardIndex` (full collection) plus a per-request "what is due now?" query. This unblocks file-watcher updates and the dashboard's live counts.

**Files:**
- Modify: `src/cmd/serve/state.rs`
- Modify: `src/cmd/serve/server.rs`
- Modify: `src/cmd/serve/get.rs`
- Modify: `src/cmd/serve/post.rs`
- Modify: `src/collection.rs`

The new shape:

```rust
pub struct CardIndex {
    pub cards: Vec<Card>,
}

pub struct AppState {
    pub port: u16,
    pub directory: PathBuf,
    pub macros: Vec<(String, String)>,
    pub session_id: i64,
    pub answer_controls: AnswerControls,
    pub config: ServeFilters,         // card_limit, new_card_limit, deck_filter, bury_siblings, shuffle
    pub cards: Arc<RwLock<CardIndex>>,
    pub db: Arc<Mutex<Database>>,
    pub session_state: Arc<Mutex<SessionState>>,
}

pub struct SessionState {
    pub reveal: bool,
}
```

The "next due card" is computed on the fly from `cards` + `db.due_today(today)` + the configured filters; we no longer maintain a pre-shuffled queue.

- [ ] **Step 1: Update `state.rs` to the new shape**

Replace `src/cmd/serve/state.rs` with:

```rust
// Copyright 2025–2026 Fernando Borretti
// (license header same as before)

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

use crate::cmd::serve::server::AnswerControls;
use crate::db::Database;
use crate::types::card::Card;

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

pub struct SessionState {
    pub reveal: bool,
}
```

- [ ] **Step 2: Add a `compute_due_queue` helper in `state.rs`**

Append to `state.rs`:

```rust
use crate::cmd::serve::server::filter_deck;
use crate::cmd::serve::server::bury_siblings;
use crate::error::Fallible;
use crate::rng::TinyRng;
use crate::rng::shuffle;
use crate::types::card_hash::CardHash;
use crate::types::date::Date;
use std::collections::HashSet;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

impl AppState {
    /// Build the live queue of due cards according to the configured filters.
    /// Caller holds no locks; this acquires read on cards and lock on db.
    pub fn compute_due_queue(&self, today: Date) -> Fallible<Vec<Card>> {
        let cards = self.cards.read().unwrap().cards.clone();
        let db = self.db.lock().unwrap();
        let due_today: HashSet<CardHash> = db.due_today(today)?;
        drop(db); // release mutex before calling filter_deck which re-locks
        let due_today: Vec<Card> = cards
            .into_iter()
            .filter(|c| due_today.contains(&c.hash()))
            .collect();
        let db = self.db.lock().unwrap();
        let due_today = filter_deck(
            &db,
            due_today,
            self.filters.card_limit,
            self.filters.new_card_limit,
            self.filters.deck_filter.clone(),
        )?;
        drop(db);
        let due_today = if self.filters.bury_siblings {
            bury_siblings(due_today)
        } else {
            due_today
        };
        let due_today = if self.filters.shuffle {
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64;
            let mut rng = TinyRng::from_seed(seed);
            shuffle(due_today, &mut rng)
        } else {
            due_today
        };
        Ok(due_today)
    }
}
```

(`filter_deck` and `bury_siblings` are functions currently in `server.rs`; they need to be `pub` for this to compile. Make them `pub fn` in `server.rs`.)

- [ ] **Step 3: Allow re-parsing the deck without re-opening the DB**

In `src/collection.rs`, add a sibling helper to `Collection::new`:

```rust
/// Re-parse the .md files in `directory` and return the new card list and
/// macros, leaving any existing Database handle untouched. Used by the
/// file watcher to refresh the in-memory card index.
pub fn parse_only(directory: &PathBuf) -> Fallible<(Vec<Card>, Vec<(String, String)>)> {
    let macros: Vec<(String, String)> = {
        let mut macros = Vec::new();
        let macros_path = directory.join("macros.tex");
        if macros_path.exists() {
            let content = read_to_string(macros_path)?;
            for line in content.lines() {
                if !line.trim_start().starts_with('%') {
                    if let Some((name, definition)) = line.split_once(' ') {
                        macros.push((name.to_string(), definition.to_string()));
                    }
                }
            }
        }
        macros
    };
    let cards = parse_deck(directory)?;
    validate_media_files(&cards, directory)?;
    Ok((cards, macros))
}
```

- [ ] **Step 4: Update `server.rs` to construct `AppState` and not pre-build the queue**

Rewrite `start_server` (in `src/cmd/serve/server.rs`):

```rust
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
        session_state: Arc::new(Mutex::new(SessionState { reveal: false })),
    };

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
```

Update imports in server.rs:

```rust
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

use crate::cmd::serve::state::AppState;
use crate::cmd::serve::state::CardIndex;
use crate::cmd::serve::state::ServeFilters;
use crate::cmd::serve::state::SessionState;
```

Make `filter_deck` and `bury_siblings` `pub`:

```rust
pub fn filter_deck(...)
pub fn bury_siblings(...)
```

- [ ] **Step 5: Update get.rs to use AppState and recompute the due queue**

In `src/cmd/serve/get.rs`, change `State<ServerState>` to `State<AppState>`. Rewrite `inner`:

```rust
async fn inner(state: AppState) -> Fallible<Markup> {
    let today = crate::types::date::Date::today();
    let queue = state.compute_due_queue(today)?;
    let session = state.session_state.lock().unwrap();
    let body = if queue.is_empty() {
        render_caught_up()
    } else {
        let card = queue.into_iter().next().unwrap();
        render_card_page(&state, &session, &card, queue_remaining(&state, today)?)?
    };
    Ok(page_template(body))
}

fn queue_remaining(state: &AppState, today: crate::types::date::Date) -> Fallible<usize> {
    Ok(state.compute_due_queue(today)?.len())
}
```

Update `render_session_page` (rename to `render_card_page`) to take `(&AppState, &SessionState, &Card, usize)`. Use the same body as the post-Task-10 version, drawing `remaining` from the parameter instead of `mutable.cards.len()`.

- [ ] **Step 6: Update post.rs to use AppState**

In `src/cmd/serve/post.rs`, change `State<ServerState>` to `State<AppState>`. Replace `state.mutable.lock()` with locking into the new pieces:

```rust
async fn action_handler(state: AppState, action: Action) -> Fallible<()> {
    match action {
        Action::Reveal => {
            state.session_state.lock().unwrap().reveal = true;
        }
        Action::Undo => {} // no-op in serve mode
        Action::Forgot | Action::Hard | Action::Good | Action::Easy => {
            let today = crate::types::date::Date::today();
            let mut session = state.session_state.lock().unwrap();
            if !session.reveal {
                return Ok(());
            }
            // Pick next card the same way the GET handler does.
            let queue = state.compute_due_queue(today)?;
            if queue.is_empty() {
                return Ok(());
            }
            let card: crate::types::card::Card = queue.into_iter().next().unwrap();
            let hash = card.hash();
            let grade = action.grade();
            let reviewed_at = Timestamp::now();

            let mut db = state.db.lock().unwrap();
            let prior: Performance = db.get_card_performance(hash)?;
            let new_perf: ReviewedPerformance = update_performance(prior, grade, reviewed_at);
            let new_perf_enum = Performance::Reviewed(new_perf);

            let record = crate::db::ReviewRecord {
                card_hash: hash,
                reviewed_at,
                grade,
                stability: new_perf.stability,
                difficulty: new_perf.difficulty,
                interval_raw: new_perf.interval_raw,
                interval_days: new_perf.interval_days,
                due_date: new_perf.due_date,
            };
            db.record_rating(state.session_id, &record, new_perf_enum)?;
            drop(db);

            session.reveal = false;
        }
    }
    Ok(())
}
```

The `Hard`/`Forgot` "should_repeat" behavior is now naturally handled: if the card is still due today (because `update_performance` keeps `due_date` as today for those grades), it shows up again on the next request.

(Verify in `src/types/performance.rs` whether `Forgot`/`Hard` schedule `due_date == today`. If they do, the loop is automatic. If not, this would need a "review queue" auxiliary structure — see step 7 below.)

- [ ] **Step 7: Decide on Hard/Forgot relapse behavior**

Read `src/types/performance.rs::update_performance` and `src/fsrs.rs::interval` to determine: when grade is `Forgot` or `Hard`, what is `interval_days`? If 0, `due_date == today` and the card naturally re-enters the queue. If 1+, the card disappears for a day — different from drill semantics.

If interval is 0 for those grades: no extra logic needed, document this in a comment.

If not: add a session-local "relapse queue" inside `SessionState`:

```rust
pub struct SessionState {
    pub reveal: bool,
    pub relapse_queue: Vec<CardHash>, // cards to re-show this server-uptime session
}
```

And modify `compute_due_queue` to surface them at the front. Defer this complexity until empirically confirmed by inspection — write a quick test:

```rust
#[test]
fn test_forgot_due_date_today() {
    use crate::fsrs::Grade;
    let now = Timestamp::now();
    let p = update_performance(Performance::New, Grade::Forgot, now);
    assert_eq!(p.due_date, now.date());
}
```

Add this test to `src/types/performance.rs`'s test module. If it fails, implement the relapse queue. If it passes, no further action.

- [ ] **Step 8: Update test fixtures in `cmd/serve/mod.rs`**

The existing tests in `cmd/serve/mod.rs` directly construct `ServerState` or use `start_server`. If they use `start_server`, the test scaffolding probably still works because `ServerConfig` is unchanged. Build, then fix any compile errors that surface.

Also remove tests for `Undo` since we made it a no-op (or repurpose them to assert no-op behavior).

Run: `cargo build 2>&1 | tail -30`
Expected: builds.

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: all pass.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Recompute due queue per request from CardIndex + DB

Replace the startup-time Vec<Card> snapshot with an Arc<RwLock<CardIndex>>
that the file watcher can rebuild. The per-request handler computes the
due set live, so newly-due cards (date advance) and newly-added cards
(file edits, once the watcher lands) appear without restart.

Also adds Collection::parse_only for re-parsing decks without re-opening
the DB — used by the watcher in the next task.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Add `notify` dependency and watcher module

**Files:**
- Modify: `Cargo.toml`
- Create: `src/cmd/serve/watcher.rs`
- Modify: `src/cmd/serve/mod.rs`

- [ ] **Step 1: Add `notify` to Cargo.toml**

In `Cargo.toml`, under `[dependencies]`, add:

```toml
notify = "8.0.0"
```

Run: `cargo build 2>&1 | tail -5`
Expected: dependency downloads and the build still succeeds.

- [ ] **Step 2: Add `mod watcher` to serve/mod.rs**

In `src/cmd/serve/mod.rs`, add `mod watcher;` next to the existing module declarations.

- [ ] **Step 3: Create `src/cmd/serve/watcher.rs`**

```rust
// Copyright 2025–2026 Fernando Borretti
// (license header)

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
                    )).into());
                }
                log::warn!("file watcher unavailable ({e}); falling back to polling.");
                // Skip notify, just spawn the poll loop below.
                spawn_poll_only(directory, cards, db, rescan_interval.unwrap());
                return Ok(());
            }
        };
        watcher.watch(&directory, RecursiveMode::Recursive)
            .map_err(|e| crate::error::ErrorReport::new(format!("watch failed: {e}")))?;
        // Keep the watcher alive for the program's lifetime.
        std::mem::forget(watcher);
    }

    // Spawn the optional polling task.
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

    // Spawn the consumer that debounces and rebuilds.
    tokio::spawn(async move {
        loop {
            // Wait for at least one event.
            if rx.recv().await.is_none() {
                break;
            }
            // Debounce: drain anything that arrives within DEBOUNCE.
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
```

- [ ] **Step 4: Wire watcher into server startup**

In `src/cmd/serve/server.rs`, after constructing the `state` value but before `let app = Router::new()`, add:

```rust
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
```

Add a small duration parser (`30s`, `5m`) to `server.rs` (or to `utils.rs` if you prefer):

```rust
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
        other => return Err(crate::error::ErrorReport::new(format!(
            "invalid duration unit: {other}"
        )).into()),
    };
    Ok(std::time::Duration::from_secs(secs))
}
```

- [ ] **Step 5: Write a watcher test (TempDir + write a file + assert update)**

Create the test in `src/cmd/serve/watcher.rs` under a `#[cfg(test)] mod tests` block:

```rust
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

        // Empty deck initially.
        let db_path = dir_path.join("hashcards.db");
        let db = Database::new(db_path.to_str().unwrap())?;
        let cards = Arc::new(RwLock::new(CardIndex { cards: vec![] }));
        let db = Arc::new(Mutex::new(db));

        spawn_watcher(dir_path.clone(), cards.clone(), db.clone(), None, true)?;

        // Drop a deck file.
        write(
            dir_path.join("Test.md"),
            "Q: foo\nA: bar\n",
        )?;

        // Wait up to 2s for the rebuild.
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if cards.read().unwrap().cards.len() > 0 {
                return Ok(());
            }
        }
        panic!("watcher did not rebuild within 2s");
    }
}
```

- [ ] **Step 6: Build and run watcher test**

Run: `cargo test --lib cmd::serve::watcher::tests 2>&1 | tail -10`
Expected: pass (give it some grace; flake risk is low at 2s timeout).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Add file watcher: notify-based with polling fallback

New watcher module spawns a notify-backed RecommendedWatcher that
debounces events over 400ms and rebuilds the in-memory card index
on .md file changes. --rescan-interval enables a polling fallback
for filesystems where inotify is silent (NFS, Docker Desktop on
Windows, etc.). --no-watch disables the watcher entirely.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Add `/healthz`, `/api/decks`, `/api/cards`, `/drill` routes

**Files:**
- Create: `src/cmd/serve/api.rs`
- Modify: `src/cmd/serve/server.rs`
- Modify: `src/cmd/serve/get.rs` (move drill view to `/drill`)

- [ ] **Step 1: Create `src/cmd/serve/api.rs`**

```rust
// Copyright 2025–2026 Fernando Borretti
// (license header)

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;

use crate::cmd::serve::state::AppState;
use crate::types::date::Date;
use crate::types::performance::Performance;

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
        let rel = c.relative_file_path(&dir).map(|p| p.display().to_string()).unwrap_or_default();
        out.push(CardSummary {
            hash: c.hash().to_hex(),
            deck: c.deck_name().to_string(),
            source_file: rel,
        });
    }
    (StatusCode::OK, Json(out))
}
```

- [ ] **Step 2: Wire routes into the Router**

In `src/cmd/serve/server.rs`, add to the Router builder chain:

```rust
        .route("/healthz", get(crate::cmd::serve::api::healthz_handler))
        .route("/api/decks", get(crate::cmd::serve::api::api_decks_handler))
        .route("/api/cards", get(crate::cmd::serve::api::api_cards_handler))
        .route("/drill", get(crate::cmd::serve::get::get_handler))
```

Move the `/` route off `get_handler` to a new dashboard handler (added in Task 15). For now (this task), keep `/` pointing at `get_handler` — it'll be replaced once the dashboard exists.

Add `mod api;` to `src/cmd/serve/mod.rs`.

- [ ] **Step 3: Add a test for `/healthz`**

In `src/cmd/serve/mod.rs` tests, add:

```rust
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
```

- [ ] **Step 4: Build, test**

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Add /healthz, /api/decks, /api/cards, /drill routes

/healthz returns 200 OK once startup completes. /api/decks and
/api/cards expose read-only metadata for future LLM-authoring tools
(no card text or review state). /drill is the per-card view the
dashboard links into; / will host the dashboard in the next task.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Add `?deck=` filter on `/drill`

**Files:**
- Modify: `src/cmd/serve/get.rs`

- [ ] **Step 1: Failing test for deck filter**

In `src/cmd/serve/mod.rs` tests, add:

```rust
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

    // Filter by a deck that exists in the test fixture (one of the .md files).
    // Adjust deck name to match your test fixture, e.g. "BasicDeck".
    let response = reqwest::get(format!(
        "http://{TEST_HOST}:{port}/drill?deck=BasicDeck"
    )).await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}
```

(The exact deck name comes from `test/` fixtures; inspect `test/` to set the right name.)

- [ ] **Step 2: Implement query extraction**

In `src/cmd/serve/get.rs`, change the handler signature to extract the query param:

```rust
use axum::extract::Query;
use serde::Deserialize;

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
```

In `inner`, override the deck filter when present:

```rust
async fn inner(state: AppState, deck: Option<String>) -> Fallible<Markup> {
    let today = crate::types::date::Date::today();
    let mut filtered_state = state.clone();
    if let Some(d) = deck {
        filtered_state.filters.deck_filter = Some(d);
    }
    let queue = filtered_state.compute_due_queue(today)?;
    // ... rest unchanged ...
}
```

- [ ] **Step 3: Run test**

Run: `cargo test --lib cmd::serve::tests::test_drill_deck_filter 2>&1 | tail -10`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Add ?deck=<name> query filter on /drill

Lets the dashboard link to drill-only-this-deck. Reuses the existing
deck_filter machinery via a per-request override on a cloned AppState.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: Implement dashboard handler

**Files:**
- Create: `src/cmd/serve/dashboard.rs`
- Modify: `src/cmd/serve/server.rs`
- Modify: `src/cmd/serve/template.rs` (if shared layout needed)

- [ ] **Step 1: Create dashboard handler with status block + caught-up state**

Create `src/cmd/serve/dashboard.rs`:

```rust
// Copyright 2025–2026 Fernando Borretti
// (license header)

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use maud::Markup;
use maud::html;

use crate::cmd::serve::state::AppState;
use crate::cmd::serve::template::page_template;
use crate::error::Fallible;
use crate::types::date::Date;

pub async fn dashboard_handler(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let html = match render_dashboard(state).await {
        Ok(m) => m,
        Err(e) => page_template(html! {
            div.error {
                h1 { "Error" }
                p { (e) }
            }
        }),
    };
    (StatusCode::OK, Html(html.into_string()))
}

async fn render_dashboard(state: AppState) -> Fallible<Markup> {
    let today = Date::today();
    let queue = state.compute_due_queue(today)?;
    let due_count = queue.len();
    let streak = state.db.lock().unwrap().current_streak(today)?;
    let last_reviewed = state.db.lock().unwrap().last_review_timestamp()?;

    let body = html! {
        div.dashboard {
            (status_block(due_count))
            (streak_block(streak))
            (footer_block(last_reviewed))
        }
    };
    Ok(page_template(body))
}

fn status_block(due_count: usize) -> Markup {
    if due_count == 0 {
        html! {
            div.status.caught-up {
                h1 { "You're caught up." }
            }
        }
    } else {
        html! {
            div.status {
                div.due-count { (due_count) " cards due" }
                a.btn-primary href="/drill" { "Drill" }
            }
        }
    }
}

fn streak_block(days: u32) -> Markup {
    if days == 0 {
        html! { div.streak { "0 day streak — start one" } }
    } else {
        html! { div.streak { (days) " day streak" } }
    }
}

fn footer_block(last: Option<crate::types::timestamp::Timestamp>) -> Markup {
    let last_str = match last {
        None => "Never drilled".to_string(),
        Some(ts) => format!("Last drilled: {}", ts.into_inner().format("%Y-%m-%d %H:%M")),
    };
    html! {
        div.footer {
            (last_str)
            " · "
            a href="/stats" { "Detailed stats →" }
        }
    }
}
```

- [ ] **Step 2: Wire dashboard route**

In `src/cmd/serve/server.rs`, change the `/` route from `get(get_handler)` to:

```rust
        .route("/", get(crate::cmd::serve::dashboard::dashboard_handler))
```

The `POST /` handler stays as the rating handler. The `/drill` route already points at `get_handler`. Browser flow: `/` shows dashboard, click Drill → `/drill` → see card → POST `/` to rate (the `Redirect::to("/drill")` in `post.rs` returns to the next card).

Add `mod dashboard;` to `src/cmd/serve/mod.rs`.

- [ ] **Step 3: Test dashboard returns 200 and contains expected text**

Add to `cmd/serve/mod.rs` tests:

```rust
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
    assert!(html.contains("Drill") || html.contains("caught up"));
    Ok(())
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Add dashboard handler at GET /

Dashboard renders status block (due count + Drill button or caught-up
state), streak counter, and footer with last-drilled timestamp.
Heatmap, per-deck breakdown, and stats grid land in subsequent tasks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 16: Dashboard heatmap (server-rendered SVG)

**Files:**
- Modify: `src/cmd/serve/dashboard.rs`

- [ ] **Step 1: Add SVG heatmap renderer**

In `src/cmd/serve/dashboard.rs`, add:

```rust
use chrono::Datelike;
use chrono::Duration;
use std::collections::HashMap;

use crate::types::date::Date;

const WEEKS: i64 = 53;
const CELL: i32 = 12;
const GAP: i32 = 2;

fn heatmap_block(state: &AppState, today: Date) -> Fallible<Markup> {
    let start = Date::new(today.into_inner() + Duration::days(-(WEEKS * 7 - 1)));
    let counts: Vec<(Date, u32)> = state
        .db
        .lock()
        .unwrap()
        .count_reviews_in_date_range(start, today)?;
    let map: HashMap<Date, u32> = counts.into_iter().collect();

    let mut svg = String::new();
    let width = WEEKS as i32 * (CELL + GAP);
    let height = 7 * (CELL + GAP);
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\" class=\"heatmap-svg\">",
        width, height, width, height
    ));
    let start_naive = start.into_inner();
    // Walk back to the prior Sunday so columns align.
    let weekday_offset = start_naive.weekday().num_days_from_sunday() as i64;
    let grid_start = start_naive - Duration::days(weekday_offset);

    for w in 0..WEEKS {
        for d in 0..7 {
            let date = grid_start + Duration::days(w * 7 + d);
            if date < start_naive || date > today.into_inner() {
                continue;
            }
            let count = map.get(&Date::new(date)).copied().unwrap_or(0);
            let class = bucket_class(count);
            let x = w as i32 * (CELL + GAP);
            let y = d as i32 * (CELL + GAP);
            svg.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" class=\"hm-cell hm-{}\"><title>{}: {}</title></rect>",
                x, y, CELL, CELL, class, date, count
            ));
        }
    }
    svg.push_str("</svg>");
    Ok(html! {
        div.heatmap {
            (maud::PreEscaped(svg))
        }
    })
}

fn bucket_class(count: u32) -> &'static str {
    match count {
        0 => "0",
        1..=5 => "1",
        6..=15 => "2",
        16..=30 => "3",
        _ => "4",
    }
}
```

Wire it into `render_dashboard`:

```rust
async fn render_dashboard(state: AppState) -> Fallible<Markup> {
    let today = Date::today();
    let queue = state.compute_due_queue(today)?;
    let due_count = queue.len();
    let streak = state.db.lock().unwrap().current_streak(today)?;
    let last_reviewed = state.db.lock().unwrap().last_review_timestamp()?;
    let heatmap = heatmap_block(&state, today)?;

    let body = html! {
        div.dashboard {
            (status_block(due_count))
            (streak_block(streak))
            (heatmap)
            (footer_block(last_reviewed))
        }
    };
    Ok(page_template(body))
}
```

- [ ] **Step 2: Build and run server, verify heatmap appears**

Run: `cargo build 2>&1 | tail -5`
Expected: builds.

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: pass (existing tests still work).

- [ ] **Step 3: Commit**

```bash
git add src/cmd/serve/dashboard.rs
git commit -m "$(cat <<'EOF'
Add server-rendered SVG heatmap to dashboard

53 weeks x 7 days, colored by review count in 5 buckets
(0, 1-5, 6-15, 16-30, 30+). Cells include <title> tooltips.
CSS for the buckets lands with the dashboard styling in Task 18.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 17: Dashboard per-deck table + stats grid

**Files:**
- Modify: `src/cmd/serve/dashboard.rs`

- [ ] **Step 1: Add helpers**

Append in `dashboard.rs`:

```rust
use crate::types::performance::Maturity;

fn per_deck_block(state: &AppState, today: Date) -> Fallible<Markup> {
    let cards = state.cards.read().unwrap().cards.clone();
    let due = state.db.lock().unwrap().due_today(today)?;
    let mut totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for c in &cards {
        let entry = totals.entry(c.deck_name().to_string()).or_insert((0, 0));
        entry.0 += 1; // total
        if due.contains(&c.hash()) {
            entry.1 += 1; // due
        }
    }
    Ok(html! {
        div.decks {
            h2 { "Decks" }
            table {
                thead {
                    tr { th { "Deck" } th { "Due" } th { "Total" } }
                }
                tbody {
                    @for (deck, (total, due)) in &totals {
                        tr.deck-row class=(if *due == 0 { "dimmed" } else { "" }) {
                            td { a href={ "/drill?deck=" (deck) } { (deck) } }
                            td { (due) }
                            td { (total) }
                        }
                    }
                }
            }
        }
    })
}

fn stats_block(state: &AppState) -> Fallible<Markup> {
    use crate::types::performance::Performance;

    let cards = state.cards.read().unwrap().cards.clone();
    let db = state.db.lock().unwrap();
    let total_cards = cards.len();
    let total_reviews = db.total_reviews()?;
    let today = Date::today();
    let reviewed_today = db.count_reviews_in_date(today)?;
    let retention = db.retention_rate(30)?;

    // Maturity distribution.
    let mut new_n = 0usize;
    let mut learn_n = 0usize;
    let mut young_n = 0usize;
    let mut mature_n = 0usize;
    for c in &cards {
        let perf: Performance = match db.get_card_performance_opt(c.hash())? {
            Some(p) => p,
            None => Performance::New,
        };
        match perf.maturity() {
            Maturity::New => new_n += 1,
            Maturity::Learning => learn_n += 1,
            Maturity::Young => young_n += 1,
            Maturity::Mature => mature_n += 1,
        }
    }
    drop(db);

    Ok(html! {
        div.stats-grid {
            div.stat-tile { div.label { "Total cards" } div.value { (total_cards) } }
            div.stat-tile { div.label { "Total reviews" } div.value { (total_reviews) } }
            div.stat-tile { div.label { "Reviewed today" } div.value { (reviewed_today) } }
            div.stat-tile { div.label { "Retention (30d)" } div.value { (format!("{:.0}%", retention * 100.0)) } }
            div.stat-tile.maturity {
                div.label { "Maturity" }
                div.value {
                    "New " (new_n)
                    " · Learning " (learn_n)
                    " · Young " (young_n)
                    " · Mature " (mature_n)
                }
            }
        }
    })
}
```

Wire both into `render_dashboard`:

```rust
async fn render_dashboard(state: AppState) -> Fallible<Markup> {
    let today = Date::today();
    let queue = state.compute_due_queue(today)?;
    let due_count = queue.len();
    let streak = state.db.lock().unwrap().current_streak(today)?;
    let last_reviewed = state.db.lock().unwrap().last_review_timestamp()?;
    let heatmap = heatmap_block(&state, today)?;
    let decks = per_deck_block(&state, today)?;
    let stats = stats_block(&state)?;

    let body = html! {
        div.dashboard {
            (status_block(due_count))
            (streak_block(streak))
            (heatmap)
            (decks)
            (stats)
            (footer_block(last_reviewed))
        }
    };
    Ok(page_template(body))
}
```

- [ ] **Step 2: Build, test**

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add src/cmd/serve/dashboard.rs
git commit -m "$(cat <<'EOF'
Add per-deck table and stats grid to dashboard

Per-deck table: deck name, due count, total count, with deck names
linking to /drill?deck=<name>. Stats grid: total cards, total reviews,
reviewed today, retention (30d), maturity distribution.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 18: Dashboard CSS — desktop and mobile layout

**Files:**
- Modify: `src/cmd/serve/style.css`

- [ ] **Step 1: Append dashboard styles**

Append to `src/cmd/serve/style.css`:

```css
/* ===== Dashboard ===== */

.dashboard {
    max-width: 1200px;
    margin: 0 auto;
    padding: 1rem;
    display: grid;
    gap: 1.25rem;
}

.dashboard .status {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.5rem;
}

.dashboard .status .due-count {
    font-size: 2.5rem;
    font-weight: 700;
}

.dashboard .status.caught-up h1 {
    font-size: 1.5rem;
    margin: 0;
}

.dashboard .btn-primary {
    display: inline-block;
    padding: 0.6rem 1.4rem;
    border-radius: 6px;
    background: var(--accent, #2563eb);
    color: #fff;
    text-decoration: none;
    font-weight: 600;
}

.dashboard .streak {
    font-size: 1.1rem;
    color: var(--text-secondary, #555);
}

.dashboard .heatmap {
    overflow-x: auto;
}

.dashboard .heatmap-svg .hm-cell { stroke: rgba(0,0,0,0.04); }
.dashboard .heatmap-svg .hm-0 { fill: #ebedf0; }
.dashboard .heatmap-svg .hm-1 { fill: #9be9a8; }
.dashboard .heatmap-svg .hm-2 { fill: #40c463; }
.dashboard .heatmap-svg .hm-3 { fill: #30a14e; }
.dashboard .heatmap-svg .hm-4 { fill: #216e39; }

.dashboard .decks table {
    width: 100%;
    border-collapse: collapse;
}
.dashboard .decks th, .dashboard .decks td {
    padding: 0.4rem 0.6rem;
    text-align: left;
    border-bottom: 1px solid rgba(0,0,0,0.08);
}
.dashboard .decks tr.dimmed { opacity: 0.55; }
.dashboard .decks a { color: inherit; text-decoration: none; }
.dashboard .decks a:hover { text-decoration: underline; }

.dashboard .stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 0.75rem;
}
.dashboard .stat-tile {
    border: 1px solid rgba(0,0,0,0.08);
    border-radius: 6px;
    padding: 0.75rem;
}
.dashboard .stat-tile .label {
    font-size: 0.85rem;
    color: var(--text-secondary, #666);
}
.dashboard .stat-tile .value {
    font-size: 1.4rem;
    font-weight: 700;
    margin-top: 0.25rem;
}
.dashboard .stat-tile.maturity .value {
    font-size: 0.95rem;
    font-weight: 500;
}

.dashboard .footer {
    color: var(--text-secondary, #666);
    font-size: 0.9rem;
    border-top: 1px solid rgba(0,0,0,0.08);
    padding-top: 0.75rem;
}

@media (min-width: 769px) {
    .dashboard {
        grid-template-columns: 1fr 1fr;
        grid-template-areas:
            "status   stats"
            "streak   stats"
            "heatmap  heatmap"
            "decks    decks"
            "footer   footer";
    }
    .dashboard .status     { grid-area: status; }
    .dashboard .streak     { grid-area: streak; }
    .dashboard .stats-grid { grid-area: stats; }
    .dashboard .heatmap    { grid-area: heatmap; }
    .dashboard .decks      { grid-area: decks; }
    .dashboard .footer     { grid-area: footer; }
}

/* Drill view tweaks */
.due-count {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.5rem 1rem;
    font-weight: 600;
}
.dashboard-link {
    text-decoration: none;
    color: var(--text-secondary, #555);
    font-weight: 400;
    font-size: 0.9rem;
}

@media (prefers-color-scheme: dark) {
    .dashboard .heatmap-svg .hm-0 { fill: #161b22; }
    .dashboard .heatmap-svg .hm-1 { fill: #0e4429; }
    .dashboard .heatmap-svg .hm-2 { fill: #006d32; }
    .dashboard .heatmap-svg .hm-3 { fill: #26a641; }
    .dashboard .heatmap-svg .hm-4 { fill: #39d353; }
}
```

- [ ] **Step 2: Build and run a manual smoke test in a browser**

Run: `cargo run -- serve test/ --no-watch --port 8765 &` (use a directory that has cards; `test/` if applicable, otherwise `example/`)

Open `http://localhost:8765/` in a browser. Verify dashboard shows up with status, streak, heatmap, decks, stats, and footer. Resize the browser to mobile width (≤768px) and verify single-column layout.

Stop the server: `pkill -f "hashcards serve"`.

- [ ] **Step 3: Commit**

```bash
git add src/cmd/serve/style.css
git commit -m "$(cat <<'EOF'
Add dashboard CSS — desktop two-column, mobile single-column

CSS grid layout: on >=769px the status/streak share the left column
with the stats grid on the right; heatmap, decks, footer span full
width. On <769px everything stacks. Includes dark-mode heatmap
palette and a horizontally-scrollable heatmap container for narrow
viewports.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 19: Implement HTML output for `stats` command

**Files:**
- Modify: `src/cmd/stats.rs`
- Modify: `src/cmd/serve/dashboard.rs` (extract reusable fragment helpers)

- [ ] **Step 1: Move fragment helpers to a shared location**

In `src/cmd/serve/dashboard.rs`, mark `heatmap_block`, `per_deck_block`, `stats_block`, `streak_block`, and `footer_block` as `pub` so they can be reused.

Adjust the route file accordingly: dashboard renders, stats command renders the same blocks (without the `Drill` button) into a static HTML page.

- [ ] **Step 2: Implement HTML stats output**

Replace the `StatsFormat::Html` arm in `src/cmd/stats.rs::print_stats`:

```rust
StatsFormat::Html => {
    use crate::cmd::serve::dashboard::heatmap_block;
    use crate::cmd::serve::dashboard::per_deck_block;
    use crate::cmd::serve::dashboard::stats_block;
    use crate::cmd::serve::dashboard::streak_block;
    use crate::cmd::serve::state::AppState;
    use crate::cmd::serve::state::CardIndex;
    use crate::cmd::serve::state::ServeFilters;
    use crate::cmd::serve::state::SessionState;
    use crate::cmd::serve::server::AnswerControls;
    use crate::cmd::serve::template::page_template;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::RwLock;
    use crate::types::date::Date;
    use crate::types::timestamp::Timestamp;

    let coll = crate::collection::Collection::new(directory)?;
    let session_id = 0; // not written to; stats is read-only
    let state = AppState {
        port: 0,
        directory: coll.directory.clone(),
        macros: coll.macros.clone(),
        session_id,
        answer_controls: AnswerControls::Full,
        filters: ServeFilters {
            card_limit: None,
            new_card_limit: None,
            deck_filter: None,
            bury_siblings: false,
            shuffle: false,
        },
        cards: Arc::new(RwLock::new(CardIndex { cards: coll.cards })),
        db: Arc::new(Mutex::new(coll.db)),
        session_state: Arc::new(Mutex::new(SessionState { reveal: false })),
    };
    let today = Date::today();
    let streak = state.db.lock().unwrap().current_streak(today)?;
    let heatmap = heatmap_block(&state, today)?;
    let decks = per_deck_block(&state, today)?;
    let stats_grid = stats_block(&state)?;
    let body = maud::html! {
        h1 { "Detailed stats" }
        (streak_block(streak))
        (heatmap)
        (stats_grid)
        (decks)
    };
    println!("{}", page_template(body).into_string());
}
```

(`Stats` struct definition can stay; `--format json` continues to use it.)

Make `dashboard` module `pub` in `src/cmd/serve/mod.rs`:

```rust
pub mod dashboard;
```

And `state` module too:

```rust
pub mod state;
pub mod server;
```

(Some of these are already `pub`; check before editing.)

- [ ] **Step 3: Test by running `hashcards stats --format html`**

Run: `cargo run -- stats --format html test/ 2>&1 | head -20`
Expected: HTML output starting with `<!DOCTYPE html>` or `<html>`.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
Implement stats --format html using shared dashboard fragments

Wires the previously-stubbed HTML stats output to the same heatmap,
streak, per-deck, and stats-grid components rendered by the serve
dashboard. Output is a static HTML page suitable for piping to a file.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 20: Add Dockerfile

**Files:**
- Create: `Dockerfile`
- Create: `.dockerignore`

- [ ] **Step 1: Write the Dockerfile**

Create `Dockerfile` at repo root:

```dockerfile
# Build stage
FROM rust:1.83-slim AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY vendor ./vendor
RUN cargo build --release --locked

# Runtime stage
FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/hashcards /usr/local/bin/hashcards
WORKDIR /cards
EXPOSE 8000
ENTRYPOINT ["hashcards"]
CMD ["serve", "/cards"]
```

(The Rust version pin should match the project's actual MSRV; check `rust-toolchain.toml` if present, or pick a recent stable.)

- [ ] **Step 2: Write `.dockerignore`**

Create `.dockerignore` at repo root:

```
target/
.git/
.github/
docs/
test/
example/
.idea/
.vscode/
*.md
docker-compose*.yml
Dockerfile
```

- [ ] **Step 3: Build the image locally**

Run: `docker build -t hashcards:dev . 2>&1 | tail -10`
Expected: build succeeds; image tagged `hashcards:dev`.

- [ ] **Step 4: Smoke test the image**

Run:
```bash
mkdir -p /tmp/cards-smoke
cp test/*.md /tmp/cards-smoke/ 2>/dev/null || cp -r example/* /tmp/cards-smoke/
docker run --rm -d --name hashcards-smoke -p 8765:8000 -v /tmp/cards-smoke:/cards hashcards:dev
sleep 2
curl -fsS http://localhost:8765/healthz
docker stop hashcards-smoke
```

Expected: `curl` returns successfully (200, no body needed).

- [ ] **Step 5: Commit**

```bash
git add Dockerfile .dockerignore
git commit -m "$(cat <<'EOF'
Add Dockerfile for self-hosting

Multi-stage build (rust:slim builder, debian:stable-slim runtime).
Includes wget in the runtime image so docker-compose healthcheck
can use /healthz. Default CMD is `serve /cards`; mount your cards
directory at /cards.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 21: Add docker-compose example

**Files:**
- Create: `examples/docker/docker-compose.yml`
- Create: `examples/docker/README.md`

- [ ] **Step 1: Create directory and compose file**

```bash
mkdir -p examples/docker
```

Create `examples/docker/docker-compose.yml`:

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

Create `examples/docker/README.md`:

```markdown
# hashcards Docker example

## Quick start

1. Build the image from the repo root:

       docker build -t hashcards:latest .

2. Edit `docker-compose.yml` and replace `/path/to/your/cards` with the
   absolute path to your card directory on the host.

3. Start:

       docker compose up -d

4. Open http://localhost:8000 in your browser.

## Windows hosts

If your cards directory is on a Windows path (not in WSL2's filesystem),
add a polling fallback to the compose `command:` field:

    command: ["serve", "/cards", "--rescan-interval", "30s"]

This is needed because inotify events from Windows-side edits don't
reliably propagate into WSL2 containers.

## Backup

The SQLite database lives at `/cards/hashcards.db` inside the
container, which means it lives in your bind-mounted host directory.
Back up that directory and you back up everything (cards + progress).
```

- [ ] **Step 2: Commit**

```bash
git add examples/
git commit -m "$(cat <<'EOF'
Add docker-compose example

Includes Windows host note for the --rescan-interval polling
fallback when bind-mounting Windows filesystem paths into WSL2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 22: Update README, CLAUDE.md, CHANGELOG

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `CHANGELOG.xml`

- [ ] **Step 1: Update CLAUDE.md**

In `CLAUDE.md`, find the "Rules" section and the line:

```
- Don't persist changes to the database during drilling. Use the cache.
```

Replace with:

```
- Serve mode persists ratings per-card. The in-memory CardIndex is a read-side index only; never use it as a write buffer.
```

- [ ] **Step 2: Update README.md tutorial section**

In `README.md`, replace `hashcards drill` with `hashcards serve` in the tutorial section. Add a new section after "Tutorial" titled "Self-hosting with Docker":

```markdown
## Self-hosting with Docker

For perpetual self-hosted access from any browser (desktop or mobile):

```bash
docker build -t hashcards:latest .
docker run -d -p 8000:8000 -v /path/to/your/cards:/cards hashcards:latest
```

Then open http://localhost:8000.

For docker-compose, see `examples/docker/`.

### Card editing

Edit your `.md` files on the host with your editor of choice. The server
watches the directory and picks up changes automatically. Sync them via
git, Syncthing, rclone, or any other tool.

### Windows hosts

If your cards directory lives on a Windows filesystem (not inside WSL2),
add `--rescan-interval 30s` to the serve command so changes are picked up
via polling — inotify doesn't propagate reliably across the WSL2 boundary.

### Backup

The SQLite database (`hashcards.db`) lives in the cards directory.
Back up the cards directory and you've backed up everything.
```

- [ ] **Step 3: Update CHANGELOG.xml**

In `CHANGELOG.xml`, find `<unreleased>` and add to its `<added>` section:

```xml
            <change author="Katina Thongvong">
                Add `serve` subcommand: a perpetual self-hosted web app
                that persists ratings per-card, picks up file edits via
                inotify, and provides a dashboard with streak, heatmap,
                per-deck breakdown, and key stats. Designed to run as a
                Docker container.
            </change>
            <change author="Katina Thongvong">
                Implement `stats --format html` output using the same
                fragments as the new dashboard.
            </change>
            <change author="Katina Thongvong">
                Add `Dockerfile` and `examples/docker/` for self-hosting.
            </change>
```

And to `<removed>` (or `<changed>` if no `<removed>` section exists; create it if needed):

```xml
            <change author="Katina Thongvong">
                Remove `drill` subcommand. Use `serve` instead, which
                provides the same web UI as a long-running service.
            </change>
```

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md README.md CHANGELOG.xml
git commit -m "$(cat <<'EOF'
Update docs for serve mode

CLAUDE.md: replace cache rule with per-rating persistence rule.
README: replace `drill` with `serve` in the tutorial; add a
Self-hosting with Docker section. CHANGELOG: note the removal
of drill, the addition of serve, the implemented HTML stats
output, and Docker support.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 23: End-to-end smoke test

**Files:**
- Modify: `src/cmd/serve/mod.rs`

- [ ] **Step 1: Add an E2E test that drives a full flow**

Append to `src/cmd/serve/mod.rs` tests:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_serve_e2e_full_flow() -> Fallible<()> {
    use std::fs::write;
    use tempfile::tempdir;

    let port = pick_unused_port().unwrap();
    let dir = tempdir()?.into_path();
    write(dir.join("Smoke.md"), "Q: 2+2\nA: 4\n\nQ: 3+3\nA: 6\n")?;
    let dir_str = dir.to_str().unwrap().to_string();
    let session_started_at = Timestamp::now();
    let config = ServerConfig {
        directory: Some(dir_str),
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

    // Dashboard reflects 2 due cards.
    let html = reqwest::get(format!("http://{TEST_HOST}:{port}/")).await?.text().await?;
    assert!(html.contains("2 cards due") || html.contains("cards due"));

    // /api/cards returns 2 entries.
    let cards: Vec<serde_json::Value> = reqwest::get(format!("http://{TEST_HOST}:{port}/api/cards"))
        .await?.json().await?;
    assert_eq!(cards.len(), 2);

    // Drive one rating.
    reqwest::Client::new()
        .post(format!("http://{TEST_HOST}:{port}/"))
        .form(&[("action", "Reveal")])
        .send().await?;
    reqwest::Client::new()
        .post(format!("http://{TEST_HOST}:{port}/"))
        .form(&[("action", "Good")])
        .send().await?;

    // /api/decks reflects updated due count (2 -> 1).
    let decks: serde_json::Value = reqwest::get(format!("http://{TEST_HOST}:{port}/api/decks"))
        .await?.json().await?;
    let smoke = decks.get("Smoke").unwrap();
    assert_eq!(smoke["due_count"].as_u64().unwrap(), 1);

    Ok(())
}
```

- [ ] **Step 2: Run E2E test**

Run: `cargo test --lib cmd::serve::tests::test_serve_e2e_full_flow 2>&1 | tail -10`
Expected: pass.

- [ ] **Step 3: Run full test suite to confirm nothing regressed**

Run: `cargo test --quiet 2>&1 | tail -10`
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/cmd/serve/mod.rs
git commit -m "$(cat <<'EOF'
Add E2E smoke test for serve mode

Drives a fresh tempdir collection through dashboard load,
/api/cards introspection, one rating cycle, and verifies
/api/decks reflects the updated due count.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 24: Manual UI verification at three viewports

**Files:** none (verification only)

- [ ] **Step 1: Build release binary**

Run: `cargo build --release 2>&1 | tail -5`
Expected: builds.

- [ ] **Step 2: Start server against the example collection**

Run: `./target/release/hashcards serve example/ --no-watch --port 8765 &`

- [ ] **Step 3: Open in browser at 360px (mobile)**

Open `http://localhost:8765/` in a browser. Use devtools to set viewport width to 360px. Verify:

- Dashboard renders single-column.
- Status block, streak, heatmap (horizontal scroll), decks table, stats grid all visible without horizontal page scroll.
- Click "Drill" → drill view renders without overflow.
- Click "← Dashboard" link returns to dashboard.

- [ ] **Step 4: Set viewport to 768px (tablet boundary)**

Verify layout transitions cleanly at the boundary; no broken regions.

- [ ] **Step 5: Set viewport to 1280px (desktop)**

Verify two-column layout: status+streak on the left, stats grid on the right; heatmap, decks, footer span full width.

- [ ] **Step 6: Test dark mode**

Toggle browser/system dark mode. Verify dashboard heatmap palette switches to the dark variants and text remains legible.

- [ ] **Step 7: Stop the server**

Run: `pkill -f "hashcards serve"`

- [ ] **Step 8: Document any UI issues observed**

If any viewport has issues (overflow, illegible text, layout breaks), file them as follow-up tasks. Otherwise, no commit needed for this task.

---

## Self-review checklist (executed by plan author after writing)

**Spec coverage:**

| Spec section | Task |
|---|---|
| Single-user, no auth | implicit (no auth code added) |
| Per-rating persistence | Task 10 |
| File watching | Task 12 |
| --rescan-interval / --no-watch fallbacks | Task 8 + 12 |
| Replace drill with serve | Tasks 7, 8, 9 |
| Dashboard | Tasks 15, 16, 17, 18 |
| /api/decks, /api/cards | Task 13 |
| /healthz | Task 13 |
| /drill?deck= filter | Task 14 |
| stats --format html implementation | Task 19 |
| Dockerfile | Task 20 |
| docker-compose example | Task 21 |
| README + CLAUDE.md + CHANGELOG | Task 22 |
| Per-rating persistence regression tests | Task 10 (test_record_rating_single_transaction) + Task 23 (E2E) |
| HTTP route tests | Tasks 13, 14, 15, 23 |
| File-watcher tests | Task 12 |
| DB query unit tests | Tasks 1–5 |
| Maturity bucketing tests | Task 6 |
| Manual UI verification | Task 24 |

All spec requirements have at least one task.

**Placeholder scan:** every code step contains the actual code; no "TBD" / "TODO" markers in step bodies.

**Type consistency:** `AppState`, `CardIndex`, `ServeFilters`, `SessionState` introduced in Task 11 and used identically through Tasks 13–19. `ServerConfig` retains the same shape after Task 8 (with `rescan_interval` and `no_watch` added). `record_rating` signature is consistent between Task 10 (definition) and Task 11 (usage in post.rs).

**One known live decision (Task 11 step 7):** confirm via inspection whether `Forgot`/`Hard` grades schedule cards for `due_date == today`. If yes, the natural per-request requeue handles relapse. If no, add a session-local relapse queue. The plan tells the implementer to write a tiny test to find out.
