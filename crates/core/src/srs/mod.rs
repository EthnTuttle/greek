use crate::db::Database;
use fsrs::{FSRS, ItemState, MemoryState, NextStates};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub front: String,
    pub back: String,
    pub chapter: i32,
    pub card_type: String,
    pub pos: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardWithState {
    pub card: Card,
    pub memory: Option<MemoryStateRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStateRow {
    pub card_id: String,
    pub stability: f32,
    pub difficulty: f32,
    pub last_review: Option<String>,
    pub due_date: String,
    pub reps: i32,
    pub lapses: i32,
}

/// Default maximum number of *new* cards introduced per UTC day.
pub const DEFAULT_DAILY_NEW_CAP: u32 = 10;

/// Per-rating due-interval preview, in wall-clock seconds from now.
/// Converted from FSRS's fractional-day intervals (`days * 86400`, rounded)
/// so callers can show human-friendly labels without day-math.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct IntervalPreview {
    pub again_secs: u32,
    pub hard_secs: u32,
    pub good_secs: u32,
    pub easy_secs: u32,
}

pub struct SrsEngine {
    fsrs: FSRS,
    desired_retention: f32,
}

impl SrsEngine {
    pub fn new() -> Self {
        // Use default FSRS parameters (empty slice triggers defaults)
        let fsrs = FSRS::new(Some(&[])).expect("default FSRS parameters should be valid");
        Self {
            fsrs,
            desired_retention: 0.9,
        }
    }

    /// Get cards that are due for review (due_date <= now), ordered by most overdue first.
    /// Propagates SQL errors instead of panicking so callers can surface a recoverable error.
    pub fn get_due_cards(
        &self,
        db: &Database,
        limit: usize,
    ) -> anyhow::Result<Vec<CardWithState>> {
        let now = chrono_now();
        let mut stmt = db.conn().prepare(
            "SELECT card_id, stability, difficulty, last_review, due_date, reps, lapses
                 FROM memory_states
                 WHERE due_date <= ?1
                 ORDER BY due_date ASC
                 LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![now, limit as i64], |row| {
            Ok(MemoryStateRow {
                card_id: row.get(0)?,
                stability: row.get(1)?,
                difficulty: row.get(2)?,
                last_review: row.get(3)?,
                due_date: row.get(4)?,
                reps: row.get(5)?,
                lapses: row.get(6)?,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            let memory = row?;
            // Build a minimal card from the memory state
            // In a real app, card content would be loaded from a separate cards table/file
            let card = Card {
                id: memory.card_id.clone(),
                front: String::new(),
                back: String::new(),
                chapter: 0,
                card_type: String::new(),
                pos: String::new(),
                tags: Vec::new(),
            };
            result.push(CardWithState {
                card,
                memory: Some(memory),
            });
        }

        Ok(result)
    }

    /// Get new cards (no memory state) for a given textbook/chapter, in order.
    /// This returns cards that do NOT yet have a memory_states row.
    /// In the real app, cards would come from a cards/vocabulary table.
    /// For now, this is a stub that returns an empty list.
    pub fn get_new_cards(
        &self,
        _db: &Database,
        _textbook: &str,
        _chapter: i32,
        _limit: usize,
    ) -> Vec<Card> {
        // In production, this would query a vocabulary/cards table
        // and filter out cards that already have memory states.
        Vec::new()
    }

    /// Preview the intervals FSRS would assign to each rating for `card_id`
    /// if it were reviewed right now. An unknown card is treated as new and
    /// still returns `Some`; `None` is returned only on error (SQL/FSRS).
    pub fn preview_intervals(&self, db: &Database, card_id: &str) -> Option<IntervalPreview> {
        let current_state = load_memory_state(db, card_id);
        let now = chrono_now();
        let elapsed_days = elapsed_days_since(current_state.as_ref(), &now);

        let memory_state = current_state.as_ref().map(|s| MemoryState {
            stability: s.stability,
            difficulty: s.difficulty,
        });

        let next_states = self
            .fsrs
            .next_states(memory_state, self.desired_retention, elapsed_days)
            .ok()?;

        Some(IntervalPreview {
            again_secs: interval_days_to_secs(next_states.again.interval),
            hard_secs: interval_days_to_secs(next_states.hard.interval),
            good_secs: interval_days_to_secs(next_states.good.interval),
            easy_secs: interval_days_to_secs(next_states.easy.interval),
        })
    }

    /// Submit a review and update the memory state
    pub fn submit_review(&self, db: &Database, card_id: &str, rating: u8) -> anyhow::Result<()> {
        let now = chrono_now();

        // Load current memory state
        let current_state: Option<MemoryStateRow> = load_memory_state(db, card_id);

        // Calculate elapsed days since last review.
        // NOTE: same-day re-reviews saturate at 0 (`days_between` uses
        // `saturating_sub` on whole-day granularity). That is intentional:
        // with sub-day learning intervals, the wall-clock gap is carried by
        // the stored due_date, while FSRS's time-decay only consumes
        // whole-day gaps.
        let elapsed_days: u32 = elapsed_days_since(current_state.as_ref(), &now);

        // Build FSRS memory state
        let memory_state = current_state.as_ref().map(|s| MemoryState {
            stability: s.stability,
            difficulty: s.difficulty,
        });

        // Get next states for all ratings
        let next_states: NextStates = self
            .fsrs
            .next_states(memory_state, self.desired_retention, elapsed_days)?;

        // Pick the state corresponding to the user's rating
        let chosen_state: &ItemState = match rating {
            1 => &next_states.again,
            2 => &next_states.hard,
            3 => &next_states.good,
            4 => &next_states.easy,
            _ => &next_states.good, // default to Good for invalid ratings
        };

        let new_stability = chosen_state.memory.stability;
        let new_difficulty = chosen_state.memory.difficulty;
        let interval_days = chosen_state.interval;

        // Compute new due date
        let due_date = add_days_to_now(&now, interval_days);

        // Update reps/lapses
        let (new_reps, new_lapses) = if let Some(ref state) = current_state {
            let reps = state.reps + 1;
            let lapses = if rating == 1 {
                state.lapses + 1
            } else {
                state.lapses
            };
            (reps, lapses)
        } else {
            let lapses = if rating == 1 { 1 } else { 0 };
            (1, lapses)
        };

        // Upsert memory state
        db.conn().execute(
            "INSERT INTO memory_states (card_id, stability, difficulty, last_review, due_date, reps, lapses)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(card_id) DO UPDATE SET
                stability = excluded.stability,
                difficulty = excluded.difficulty,
                last_review = excluded.last_review,
                due_date = excluded.due_date,
                reps = excluded.reps,
                lapses = excluded.lapses",
            rusqlite::params![
                card_id,
                new_stability,
                new_difficulty,
                now,
                due_date,
                new_reps,
                new_lapses,
            ],
        )?;

        // Log the review
        let scheduled_days = interval_days as f64;
        db.conn().execute(
            "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![card_id, now, rating as i32, elapsed_days as f64, scheduled_days],
        )?;

        Ok(())
    }
}

impl Default for SrsEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the current time as an ISO 8601 string (UTC), e.g.
/// "2026-07-15T12:00:00Z". Public so other layers (UniFFI stats,
/// card creation) can produce due-date strings in the SAME format
/// that `get_due_cards` compares against.
pub fn now_iso8601() -> String {
    chrono_now()
}

/// Count of cards introduced (first-ever review) on today's UTC date.
/// A card counts as "new today" iff its EARLIEST `review_logs` timestamp
/// falls on the current UTC date; cards first reviewed on an earlier day do
/// not count even if reviewed again today. Returns 0 if the query fails.
pub fn new_cards_today(db: &Database) -> u32 {
    let now = chrono_now();
    let today = &now[..10]; // "YYYY-MM-DD"
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT MIN(timestamp) AS first_ts
                 FROM review_logs
                 GROUP BY card_id
             ) WHERE substr(first_ts, 1, 10) = ?1",
            [today],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as u32
}

/// Remaining new-card allowance for today: the daily cap minus the number of
/// cards already introduced today (saturates at 0 if the cap was exceeded).
pub fn new_cards_remaining_today(db: &Database) -> u32 {
    DEFAULT_DAILY_NEW_CAP.saturating_sub(new_cards_today(db))
}

/// Load the stored memory state for `card_id`, or `None` if the card has
/// never been reviewed (unknown rows are NOT an error here).
fn load_memory_state(db: &Database, card_id: &str) -> Option<MemoryStateRow> {
    db.conn()
        .query_row(
            "SELECT card_id, stability, difficulty, last_review, due_date, reps, lapses
             FROM memory_states WHERE card_id = ?1",
            [card_id],
            |row| {
                Ok(MemoryStateRow {
                    card_id: row.get(0)?,
                    stability: row.get(1)?,
                    difficulty: row.get(2)?,
                    last_review: row.get(3)?,
                    due_date: row.get(4)?,
                    reps: row.get(5)?,
                    lapses: row.get(6)?,
                })
            },
        )
        .ok()
}

/// Whole days between the stored `last_review` and `now` (0 for new cards
/// and for any same-day re-review — see note in `submit_review`).
fn elapsed_days_since(state: Option<&MemoryStateRow>, now: &str) -> u32 {
    state
        .and_then(|s| s.last_review.as_deref())
        .map(|last_review| days_between(last_review, now))
        .unwrap_or(0)
}

/// Returns the current time as an ISO 8601 string (UTC).
/// Uses a simple approach without external chrono dependency.
fn chrono_now() -> String {
    secs_to_iso(system_now_secs())
}

/// Current system time, seconds since the Unix epoch.
fn system_now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Convert seconds since the Unix epoch to an ISO 8601 UTC string,
/// "YYYY-MM-DDTHH:MM:SSZ".
fn secs_to_iso(secs: u64) -> String {
    let days = secs / 86400;
    let remaining_secs = secs % 86400;
    let hours = remaining_secs / 3600;
    let minutes = (remaining_secs % 3600) / 60;
    let seconds = remaining_secs % 60;

    // Simple date calculation from days since epoch (1970-01-01)
    let (year, month, day) = days_to_ymd(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch to (year, month, day)
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Parse an ISO 8601 date string and compute days between two dates.
fn days_between(from: &str, to: &str) -> u32 {
    let from_days = iso_to_days(from).unwrap_or(0);
    let to_days = iso_to_days(to).unwrap_or(0);
    to_days.saturating_sub(from_days) as u32
}

/// Parse ISO 8601 date to days since epoch (approximate, for scheduling)
fn iso_to_days(iso: &str) -> Option<u64> {
    // Parse "YYYY-MM-DDT..." format
    if iso.len() < 10 {
        return None;
    }
    let year: u64 = iso[0..4].parse().ok()?;
    let month: u64 = iso[5..7].parse().ok()?;
    let day: u64 = iso[8..10].parse().ok()?;

    // Convert to days since epoch using inverse of days_to_ymd
    Some(ymd_to_days(year, month, day))
}

/// Parse an ISO 8601 timestamp to seconds since the Unix epoch.
/// The time component is optional: legacy midnight rows like
/// "2026-07-19T00:00:00Z" parse naturally, and a missing or truncated time
/// defaults to 00:00:00 UTC so date-valid input never fails to parse.
fn iso_to_secs(iso: &str) -> Option<u64> {
    let days = iso_to_days(iso)?;
    // Slice with `get` so short/non-ASCII input degrades to 0 instead of panicking.
    let hours: u64 = iso.get(11..13).and_then(|s| s.parse().ok()).unwrap_or(0);
    let minutes: u64 = iso.get(14..16).and_then(|s| s.parse().ok()).unwrap_or(0);
    let seconds: u64 = iso.get(17..19).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(days * 86400 + hours * 3600 + minutes * 60 + seconds)
}

/// Convert an FSRS interval (days, possibly fractional) to whole seconds:
/// `days * 86400` rounded to the nearest second, negative clamped to 0
/// (FSRS intervals are never negative; used for preview values which must
/// fit the `u32` fields of `IntervalPreview` — max stability 36500 d fits).
fn interval_days_to_secs(days: f32) -> u32 {
    (days as f64 * 86400.0).round().max(0.0) as u32
}

/// Convert (year, month, day) to days since Unix epoch
fn ymd_to_days(year: u64, month: u64, day: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Add an FSRS interval (days, possibly fractional) to `now` and return a
/// full "YYYY-MM-DDTHH:MM:SSZ" timestamp. The interval is converted to
/// seconds (`days * 86400`, rounded) and added to the current instant —
/// deliberately with NO whole-day rounding, NO one-day minimum, and NO
/// midnight stamping, so sub-day learning steps (an "Again" card reappearing
/// minutes or hours later) are preserved. The result stays lexicographically
/// comparable with legacy midnight-stamped rows like "2026-07-19T00:00:00Z".
fn add_days_to_now(now: &str, days: f32) -> String {
    // `now` normally comes from `chrono_now` and always parses; fall back to
    // the real clock if a caller ever hands us something else.
    let now_secs = iso_to_secs(now).unwrap_or_else(system_now_secs);
    let interval_secs = (days as f64 * 86400.0).round() as i64;
    let new_secs = (now_secs as i64).saturating_add(interval_secs).max(0) as u64;
    secs_to_iso(new_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srs_engine_creation() {
        let engine = SrsEngine::new();
        assert!((engine.desired_retention - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_submit_review_new_card() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // Submit a review for a new card (no prior state)
        engine.submit_review(&db, "test-card-1", 3).unwrap();

        // Verify memory state was created
        let stability: f32 = db
            .conn()
            .query_row(
                "SELECT stability FROM memory_states WHERE card_id = ?1",
                ["test-card-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stability > 0.0);

        // Verify review was logged
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM review_logs WHERE card_id = ?1",
                ["test-card-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_submit_review_existing_card() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // First review
        engine.submit_review(&db, "test-card-2", 3).unwrap();
        // Second review
        engine.submit_review(&db, "test-card-2", 4).unwrap();

        // Verify reps incremented
        let reps: i32 = db
            .conn()
            .query_row(
                "SELECT reps FROM memory_states WHERE card_id = ?1",
                ["test-card-2"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reps, 2);
    }

    #[test]
    fn test_get_due_cards() {
        let db = Database::open(":memory:", None).unwrap();

        // Insert a card that's already due
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, last_review, due_date, reps, lapses)
                 VALUES ('due-card', 5.0, 5.0, '2020-01-01T00:00:00Z', '2020-01-06T00:00:00Z', 1, 0)",
                [],
            )
            .unwrap();

        // Insert a card that's not yet due (far future)
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, last_review, due_date, reps, lapses)
                 VALUES ('future-card', 5.0, 5.0, '2020-01-01T00:00:00Z', '2099-01-01T00:00:00Z', 1, 0)",
                [],
            )
            .unwrap();

        let engine = SrsEngine::new();
        let due = engine.get_due_cards(&db, 10).unwrap();

        // Only the overdue card should be returned
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].card.id, "due-card");
    }

    /// Insert a memory_states row with the given due timestamp.
    fn insert_state(db: &Database, card_id: &str, last_review: &str, due_date: &str) {
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, last_review, due_date, reps, lapses)
                 VALUES (?1, 5.0, 5.0, ?2, ?3, 1, 0)",
                rusqlite::params![card_id, last_review, due_date],
            )
            .unwrap();
    }

    fn now_plus_secs(delta: i64) -> String {
        let now_secs = iso_to_secs(&chrono_now()).unwrap() as i64;
        secs_to_iso((now_secs + delta).max(0) as u64)
    }

    #[test]
    fn test_get_due_cards_returns_subday_due() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // Card due 3 minutes ago (sub-day, same calendar day)
        insert_state(
            &db,
            "subday-card",
            &now_plus_secs(-3900),
            &now_plus_secs(-180),
        );

        let due = engine.get_due_cards(&db, 10).unwrap();
        assert!(
            due.iter().any(|c| c.card.id == "subday-card"),
            "sub-day-due learning card must be returned"
        );
    }

    #[test]
    fn test_get_due_cards_excludes_future_card() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // Card due in +2 days
        let future_due = now_plus_secs(2 * 86400);
        insert_state(&db, "future-card-2d", &chrono_now(), &future_due);

        let due = engine.get_due_cards(&db, 10).unwrap();
        assert!(
            !due.iter().any(|c| c.card.id == "future-card-2d"),
            "card due in 2 days must not be returned"
        );
    }

    #[test]
    fn test_legacy_midnight_row_parses_and_is_due() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // Legacy row stamped at midnight UTC by the old add_days_to_now.
        // It must still parse and compare correctly (lexicographically it is
        // in the past, so it is due).
        let legacy = "2026-07-19T00:00:00Z";
        insert_state(&db, "legacy-card", "2026-07-18T00:00:00Z", legacy);

        // Parse check: midnight row equals its date at 00:00:00.
        assert_eq!(
            iso_to_secs(legacy).unwrap(),
            ymd_to_days(2026, 7, 19) * 86400
        );

        let due = engine.get_due_cards(&db, 10).unwrap();
        assert!(
            due.iter().any(|c| c.card.id == "legacy-card"),
            "legacy midnight-stamped row must still parse and be due"
        );
    }

    #[test]
    fn test_preview_intervals_new_card_ordering() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // Unknown card id is treated as a brand-new card and still returns Some.
        let preview = engine
            .preview_intervals(&db, "never-reviewed-card")
            .expect("unknown card should still yield a preview");

        assert!(
            preview.again_secs <= preview.hard_secs
                && preview.hard_secs <= preview.good_secs
                && preview.good_secs <= preview.easy_secs,
            "intervals must be ordered again <= hard <= good <= easy: {:?}",
            preview
        );
    }

    #[test]
    fn test_new_cards_today_and_remaining() {
        let db = Database::open(":memory:", None).unwrap();

        // Empty DB: nothing introduced yet, full cap remaining.
        assert_eq!(new_cards_today(&db), 0);
        assert_eq!(new_cards_remaining_today(&db), DEFAULT_DAILY_NEW_CAP);

        let now = chrono_now();
        let today = &now[..10];
        let yesterday = add_days_to_now(&now, -1.0)[..10].to_string();

        let insert_log = |card_id: &str, ts: &str| {
            db.conn()
                .execute(
                    "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                     VALUES (?1, ?2, 3, 0.0, 0.0)",
                    rusqlite::params![card_id, ts],
                )
                .unwrap();
        };

        // First reviewed yesterday, again today -> NOT new today
        insert_log("old-card", &format!("{yesterday}T08:00:00Z"));
        insert_log("old-card", &format!("{today}T00:00:01Z"));
        // First reviewed today (multiple logs same day -> counts once)
        insert_log("new-a", &format!("{today}T00:00:01Z"));
        insert_log("new-a", &format!("{today}T23:59:59Z"));
        // Another card first reviewed today
        insert_log("new-b", &format!("{today}T12:00:00Z"));

        assert_eq!(new_cards_today(&db), 2);
        assert_eq!(
            new_cards_remaining_today(&db),
            DEFAULT_DAILY_NEW_CAP - 2
        );
    }

    #[test]
    fn test_add_days_to_now_preserves_time_and_subday() {
        // Whole intervals keep the time of day (no midnight stamping)
        assert_eq!(
            add_days_to_now("2024-01-01T12:30:00Z", 1.0),
            "2024-01-02T12:30:00Z"
        );
        // Fractional days become sub-day offsets (6 h)
        assert_eq!(
            add_days_to_now("2024-01-01T12:00:00Z", 0.25),
            "2024-01-01T18:00:00Z"
        );
        // Sub-day offset crossing midnight
        assert_eq!(
            add_days_to_now("2024-01-01T23:00:00Z", 0.125),
            "2024-01-02T02:00:00Z"
        );
        // Month/leap-day rollover still works
        assert_eq!(
            add_days_to_now("2024-01-31T12:00:00Z", 1.0),
            "2024-02-01T12:00:00Z"
        );
        assert_eq!(
            add_days_to_now("2024-02-28T12:00:00Z", 1.0),
            "2024-02-29T12:00:00Z"
        );
        // No one-day minimum: sub-minute intervals stay sub-minute
        let tiny = add_days_to_now("2024-06-01T00:00:00Z", 30.0 / 86400.0);
        assert_eq!(tiny, "2024-06-01T00:00:30Z");
    }

    #[test]
    fn test_submit_review_again_schedules_subday_due() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        let before_secs = iso_to_secs(&chrono_now()).unwrap();
        engine.submit_review(&db, "again-card", 1).unwrap();

        let due_date: String = db
            .conn()
            .query_row(
                "SELECT due_date FROM memory_states WHERE card_id = ?1",
                ["again-card"],
                |row| row.get(0),
            )
            .unwrap();

        let due_secs = iso_to_secs(&due_date).expect("due_date must be a full ISO timestamp");
        let gap = due_secs as i64 - before_secs as i64;

        // The old implementation rounded up to the next midnight; the fixed
        // one must place an "Again" review back within today (sub-day gap).
        assert!(gap > 0, "due date must be in the future, got gap {gap}s");
        assert!(
            gap < 86400,
            "Again-rated card must be due within a day, got gap {gap}s"
        );
        assert!(
            !due_date.ends_with("T00:00:00Z"),
            "due_date must not be midnight-stamped: {due_date}"
        );
    }

    #[test]
    fn test_days_to_ymd_and_back() {
        // 1970-01-01 = day 0
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        // Round-trip
        assert_eq!(ymd_to_days(1970, 1, 1), 0);
        assert_eq!(ymd_to_days(2024, 6, 15), iso_to_days("2024-06-15").unwrap());
    }

    #[test]
    fn test_iso_to_days() {
        let d1 = iso_to_days("2024-01-01T00:00:00Z").unwrap();
        let d2 = iso_to_days("2024-01-02T00:00:00Z").unwrap();
        assert_eq!(d2 - d1, 1);
    }
}
