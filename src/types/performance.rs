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

use chrono::Duration;
use chrono::NaiveDate;

use crate::fsrs::Difficulty;
use crate::fsrs::Grade;
use crate::fsrs::Interval;
use crate::fsrs::Recall;
use crate::fsrs::Stability;
use crate::fsrs::initial_difficulty;
use crate::fsrs::initial_stability;
use crate::fsrs::interval;
use crate::fsrs::new_difficulty;
use crate::fsrs::new_stability;
use crate::fsrs::retrievability;
use crate::types::date::Date;
use crate::types::timestamp::Timestamp;

/// The desired recall probability.
const TARGET_RECALL: f64 = 0.9;

/// The minimum review interval in days.
const MIN_INTERVAL: f64 = 1.0;

/// The maximum review interval in days.
const MAX_INTERVAL: f64 = 256.0;

/// Represents performance information for a card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Performance {
    /// The card is new, and has never been reviewed.
    New,
    /// The card has been reviewed at least once.
    Reviewed(ReviewedPerformance),
}

impl Performance {
    pub fn is_new(&self) -> bool {
        matches!(self, Performance::New)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReviewedPerformance {
    /// The timestamp when the card was last reviewed.
    pub last_reviewed_at: Timestamp,
    /// The card's stability (an FSRS parameter).
    pub stability: Stability,
    /// The card's difficulty (an FSRS parameter).
    pub difficulty: Difficulty,
    /// The FSRS-calculated interval in hours until the next review. This is
    /// the raw interval, before any rounding and clamping.
    pub interval_raw: Interval,
    /// The FSRS interval as an integer number of days.
    pub interval_days: i64,
    /// The card's next due date.
    pub due_date: Date,
    /// The number of times the card has been reviewed.
    pub review_count: usize,
}

pub fn update_performance(
    perf: Performance,
    grade: Grade,
    reviewed_at: Timestamp,
) -> ReviewedPerformance {
    let today: NaiveDate = reviewed_at.date().into_inner();
    let (stability, difficulty, review_count): (Stability, Difficulty, usize) = match perf {
        Performance::New => (initial_stability(grade), initial_difficulty(grade), 0),
        Performance::Reviewed(ReviewedPerformance {
            last_reviewed_at,
            stability,
            difficulty,
            review_count,
            ..
        }) => {
            let last_reviewed_at: NaiveDate = last_reviewed_at.date().into_inner();
            let time: Interval = (today - last_reviewed_at).num_days() as f64;
            let retr: Recall = retrievability(time, stability);
            let stability: Stability = new_stability(difficulty, stability, retr, grade);
            let difficulty: Difficulty = new_difficulty(difficulty, grade);
            (stability, difficulty, review_count)
        }
    };
    let interval_raw: Interval = interval(TARGET_RECALL, stability);
    let interval_rounded: Interval = interval_raw.round();
    let interval_clamped: Interval = interval_rounded.clamp(MIN_INTERVAL, MAX_INTERVAL);
    let interval_days: i64 = interval_clamped as i64;
    let interval_duration: Duration = Duration::days(interval_days);
    let due_date: Date = Date::new(today + interval_duration);
    ReviewedPerformance {
        last_reviewed_at: reviewed_at,
        stability,
        difficulty,
        interval_raw,
        interval_days,
        due_date,
        review_count: review_count + 1,
    }
}

/// Card maturity bucket for stats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Maturity {
    New,
    Learning,
    Young,
    Mature,
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

#[cfg(test)]
mod forgot_relapse_tests {
    use super::*;

    /// Verify that Forgot and Hard always schedule due_date to TOMORROW (not
    /// today). This confirms why the relapse_queue is necessary: the DB will
    /// not return these cards from due_today() on the same day they are rated.
    #[test]
    fn test_forgot_due_date_is_tomorrow() {
        use crate::fsrs::Grade;
        let now = Timestamp::now();
        let today = now.date();
        let p = update_performance(Performance::New, Grade::Forgot, now);
        // MIN_INTERVAL = 1.0, so interval_days is at least 1.
        assert!(p.interval_days >= 1, "Forgot interval_days should be >= 1");
        assert!(p.due_date > today, "Forgot due_date should be after today");
    }

    #[test]
    fn test_hard_due_date_is_tomorrow() {
        use crate::fsrs::Grade;
        let now = Timestamp::now();
        let today = now.date();
        let p = update_performance(Performance::New, Grade::Hard, now);
        assert!(p.interval_days >= 1, "Hard interval_days should be >= 1");
        assert!(p.due_date > today, "Hard due_date should be after today");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-2
    }

    #[test]
    fn test_new() {
        assert!(Performance::New.is_new());
        let reviewed_at = Timestamp::now();
        let reviewed_perf = update_performance(Performance::New, Grade::Good, reviewed_at);
        assert!(!Performance::Reviewed(reviewed_perf).is_new());
    }

    #[test]
    fn test_update_new_card() {
        let reviewed_at = Timestamp::now();
        let result = update_performance(Performance::New, Grade::Good, reviewed_at);
        let ReviewedPerformance {
            last_reviewed_at,
            stability,
            difficulty,
            interval_raw,
            interval_days,
            due_date: _,
            review_count,
        } = result;
        assert_eq!(last_reviewed_at, reviewed_at);
        assert!(approx_eq(stability, 3.17));
        assert!(approx_eq(difficulty, 5.28));
        assert!(approx_eq(interval_raw, 3.17));
        assert_eq!(interval_days, 3);
        assert_eq!(review_count, 1);
    }

    #[test]
    fn test_update_already_reviewed_card() {
        let now = Timestamp::now();
        let today = now.date();
        let duration = Duration::days(3);
        let last_reviewed_at = Timestamp::new(now.into_inner() - duration);
        let initial_perf = ReviewedPerformance {
            last_reviewed_at,
            stability: 3.17,
            difficulty: 5.28,
            interval_raw: 3.17,
            interval_days: 3,
            due_date: Date::new(today.into_inner() + duration),
            review_count: 1,
        };
        let reviewed_at = now;
        let result = update_performance(
            Performance::Reviewed(initial_perf),
            Grade::Easy,
            reviewed_at,
        );
        let ReviewedPerformance {
            last_reviewed_at,
            stability,
            difficulty,
            interval_raw,
            interval_days,
            due_date: _,
            review_count,
        } = result;
        assert_eq!(last_reviewed_at, reviewed_at);
        assert!(approx_eq(stability, 25.80));
        assert!(approx_eq(difficulty, 4.50));
        assert!(approx_eq(interval_raw, 25.80));
        assert_eq!(interval_days, 26);
        assert_eq!(review_count, 2);
    }
}
