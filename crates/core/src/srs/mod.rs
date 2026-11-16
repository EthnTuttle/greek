use crate::db::Database;
use fsrs::{ItemState, MemoryState, NextStates, FSRS};
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

// --- Learning-step ladder (C6) -------------------------------------------
// For cards still in (re)learning, FSRS's retention-interval `due` is
// overridden by a fixed step ladder expressed as DURATION-FROM-NOW, as
// promised by docs/guide.html:109-114 ("Misses come back the same session
// ... a card you rate Again on its first lapses is due again in minutes").
// Without this, FSRS schedules a NEW card rated Again ~0.40 days (≈9h40m)
// out — it can never resurface in the session it failed in.
// "In (re)learning" is history-derived: `reps == lapses`, i.e. every review
// so far was Again (a brand-new card trivially so). Graduated is the exact
// complement used by chapter pacing. Any non-Again rating graduates the card;
// Easy is deliberately not on the ladder (it graduates immediately). EVERY
// Again gets a one-minute relearning due step (P2.14), while preserving the
// FSRS Again memory state even for a graduated card.
const STEP_AGAIN_SECS: u64 = 60; // 1 min
const STEP_HARD_SECS: u64 = 300; // 5 min
const STEP_GOOD_SECS: u64 = 600; // 10 min

/// Injectable clock (C6): every scheduling read of "now" goes through this
/// so tests can pin/advance time. Bindings keep the system clock via
/// [`SrsEngine::new`]; the sync fold does NOT use it — it replays against
/// each event's own timestamp via the pure [`step_due`].
pub(crate) trait Clock: Send + Sync {
    /// Current time as an ISO 8601 UTC string ("YYYY-MM-DDTHH:MM:SSZ").
    fn now(&self) -> String;
}

/// The production clock: reads the system time ([`chrono_now`]).
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> String {
        chrono_now()
    }
}

pub struct SrsEngine {
    fsrs: FSRS,
    desired_retention: f32,
    clock: Box<dyn Clock>,
}

impl SrsEngine {
    pub fn new() -> Self {
        Self::with_clock(Box::new(SystemClock))
    }

    /// Engine reading "now" from an arbitrary clock (tests; sync-fold
    /// determinism experiments). Binding-facing API is [`SrsEngine::new`].
    pub(crate) fn with_clock(clock: Box<dyn Clock>) -> Self {
        // Use default FSRS parameters (empty slice triggers defaults)
        let fsrs = FSRS::new(Some(&[])).expect("default FSRS parameters should be valid");
        Self {
            fsrs,
            desired_retention: 0.9,
            clock,
        }
    }

    /// Get cards that are due for review (due_date <= now), ordered by most overdue first.
    /// Propagates SQL errors instead of panicking so callers can surface a recoverable error.
    pub fn get_due_cards(&self, db: &Database, limit: usize) -> anyhow::Result<Vec<CardWithState>> {
        let now = self.clock.now();
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
    /// still returns `Ok(Some(_))`; `Ok(None)` only when FSRS has no next
    /// states, and SQL/decode errors propagate as `Err` (never collapsed
    /// into `None`, so a corrupt stored state cannot look like a new card).
    pub fn preview_intervals(
        &self,
        db: &Database,
        card_id: &str,
    ) -> anyhow::Result<Option<IntervalPreview>> {
        let current_state = load_memory_state(db, card_id)?;
        let now = self.clock.now();
        let elapsed_days = elapsed_days_since(current_state.as_ref(), &now);

        let memory_state = current_state.as_ref().map(|s| MemoryState {
            stability: s.stability,
            difficulty: s.difficulty,
        });

        let next_states =
            self.fsrs
                .next_states(memory_state, self.desired_retention, elapsed_days)?;

        let mut preview = IntervalPreview {
            again_secs: interval_days_to_secs(next_states.again.interval),
            hard_secs: interval_days_to_secs(next_states.hard.interval),
            good_secs: interval_days_to_secs(next_states.good.interval),
            easy_secs: interval_days_to_secs(next_states.easy.interval),
        };
        // Mirror the actual scheduling policy: while the card is in
        // (re)learning the buttons must show the step ladder, not FSRS's
        // retention intervals (otherwise "Again: 9h" would be a lie).
        let (prior_reps, prior_lapses) = current_state
            .as_ref()
            .map(|s| (s.reps, s.lapses))
            .unwrap_or((0, 0));
        // P2.14: EVERY Again has a one-minute relearning step, including a
        // lapse on a graduated card. Other step previews apply only while
        // learning; all ratings still retain their FSRS next memory state.
        preview.again_secs = STEP_AGAIN_SECS as u32;
        if is_learning(prior_reps, prior_lapses) {
            preview.hard_secs = STEP_HARD_SECS as u32;
            preview.good_secs = STEP_GOOD_SECS as u32;
            // easy: graduates immediately, schedules via FSRS.
        }

        Ok(Some(preview))
    }

    /// Submit a review and update the memory state
    pub fn submit_review(&self, db: &Database, card_id: &str, rating: u8) -> anyhow::Result<()> {
        self.submit_review_with_source(db, card_id, rating, "flashcard")
    }

    /// Submit a review tagged with its origin (`source`), e.g. "flashcard"
    /// for the self-rated review flow or "practice" for measured-assessment
    /// questions. The tag lands in `review_logs.source` so accuracy metrics
    /// can be computed per activity type. A practice miss on a graduated card
    /// preserves its prior FSRS memory while retaining the normal lapse count
    /// and one-minute relearning step; every other rating/source combination
    /// uses the selected FSRS next state.
    pub fn submit_review_with_source(
        &self,
        db: &Database,
        card_id: &str,
        rating: u8,
        source: &str,
    ) -> anyhow::Result<()> {
        let now = self.clock.now();

        // Load current memory state. Errors propagate (C8): a corrupt row
        // must fail loudly rather than be mistaken for "no state" — which
        // would silently replace the history below (reps 8 -> reps 1).
        let current_state: Option<MemoryStateRow> = load_memory_state(db, card_id)?;

        // Calculate elapsed days since last review.
        // NOTE: same-day re-reviews saturate at 0 (`days_between` uses
        // `saturating_sub` on whole-day granularity). That is intentional:
        // with sub-day learning intervals, the wall-clock gap is carried by
        // the stored due_date, while FSRS's time-decay only consumes
        // whole-day gaps.
        let elapsed_days: u32 = elapsed_days_since(current_state.as_ref(), &now);

        let (prior_reps, prior_lapses) = current_state
            .as_ref()
            .map(|s| (s.reps, s.lapses))
            .unwrap_or((0, 0));

        // Build FSRS memory state
        let memory_state = current_state.as_ref().map(|s| MemoryState {
            stability: s.stability,
            difficulty: s.difficulty,
        });

        // Get next states for all ratings
        let next_states: NextStates =
            self.fsrs
                .next_states(memory_state, self.desired_retention, elapsed_days)?;

        // Pick the state corresponding to the user's rating
        let chosen_state: &ItemState = match rating {
            1 => &next_states.again,
            2 => &next_states.hard,
            3 => &next_states.good,
            4 => &next_states.easy,
            _ => &next_states.good, // default to Good for invalid ratings
        };

        let (new_stability, new_difficulty) = if preserve_memory_for_practice_miss(
            source,
            rating,
            prior_reps,
            prior_lapses,
        ) {
            let prior = current_state
                .as_ref()
                .expect("graduated practice miss requires a prior memory state");
            (prior.stability, prior.difficulty)
        } else {
            (chosen_state.memory.stability, chosen_state.memory.difficulty)
        };
        let interval_days = chosen_state.interval;

        // Compute new due date (C6): through the PURE `step_due` — history
        // (prior reps/lapses), rating, and the review timestamp go in; no
        // wall clock. The sync fold replays this exact function against
        // each event's timestamp, so both paths schedule identically.
        let due_date = step_due(prior_reps, prior_lapses, rating, interval_days, &now);

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

        // Upsert memory state + insert the review log + advance the HLC in
        // ONE transaction (C1): previously these were separate autocommit
        // statements, so a fault on the log insert (e.g. a trigger firing)
        // left the memory state advanced with no review log, and a retry
        // would double-advance. Now any failure rolls back all three.
        // Log the ACTUAL applied interval (a learning-step ladder value may
        // differ from FSRS's `interval_days`); fall back to the FSRS value
        // only if an unparsable timestamp makes the diff impossible.
        let scheduled_days = match (iso_to_secs(&due_date), iso_to_secs(&now)) {
            (Some(due), Some(now)) => due.saturating_sub(now) as f64 / 86400.0,
            _ => interval_days as f64,
        };
        db.with_tx(|conn| {
            conn.execute(
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

            // Log the review, tagged with its source activity and stamped for
            // sync (globally-unique uuid, originating device, hybrid logical
            // clock) so it can merge into other devices' databases by uuid-dedup.
            // device_id/hlc_now run on this same connection while the
            // transaction is open, so the HLC advance rolls back with the
            // rest on failure.
            conn.execute(
                "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days, source, uuid, device_id, hlc)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    card_id,
                    now,
                    rating as i32,
                    elapsed_days as f64,
                    scheduled_days,
                    source,
                    uuid::Uuid::new_v4().to_string(),
                    crate::sync::device_id(db),
                    crate::sync::hlc_now(db).to_string(),
                ],
            )?;

            Ok(())
        })?;

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

/// Count of VOCABULARY cards counting against today's new-introduction cap.
/// A non-letters card counts when its EARLIEST `review_logs` timestamp falls
/// on the current UTC date; later reviews never introduce it again. Letters
/// are exempt from the cap and therefore excluded here, while their complete
/// review history remains in `review_logs` for organic-gate evidence.
pub fn new_cards_today(db: &Database) -> u32 {
    let now = chrono_now();
    let today = &now[..10]; // "YYYY-MM-DD"
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT card_id, MIN(timestamp) AS first_ts
                 FROM review_logs
                 GROUP BY card_id
             ) WHERE substr(first_ts, 1, 10) = ?1
               AND card_id NOT LIKE 'letters-%'",
            [today],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as u32
}

/// Remaining VOCABULARY new-card allowance for today: the daily cap minus
/// non-letters first reviews today (saturates at 0). Letters do not consume
/// this allowance and are admitted independently of it.
pub fn new_cards_remaining_today(db: &Database) -> u32 {
    DEFAULT_DAILY_NEW_CAP.saturating_sub(new_cards_today(db))
}

/// Single-consultation daily-budget snapshot for building ONE review
/// session (C3).
///
/// The per-chapter `get_new_cards(textbook, chapter, limit)` path consults
/// [`new_cards_remaining_today`] independently on every call, so a caller
/// looping chapters (the old Android session builder) saw the SAME unspent
/// budget on every chapter — a 10/day cap admitted a 7+10+3 multi-curriculum
/// session. `SessionBudget` consults the remaining allowance ONCE and hands
/// out VOCABULARY admissions one card at a time, so one whole session build
/// can never exceed the day's true remainder. Letters bypass consumption via
/// [`SessionBudget::try_admit_for_textbook`].
///
/// Race note: this is a planning SNAPSHOT, not a reservation. The durable
/// `new_cards_today` counter only advances when a review is actually
/// persisted by `submit_review` (atomic upsert + log + HLC since C1), so a
/// session abandoned mid-way leaves the budget unspent, and two builders
/// started before any submission can plan from the same snapshot. That is
/// accepted for a single-user UI; the hard record of what counts remains
/// `review_logs`, written only on successful submission.
#[derive(Debug, Clone, Copy)]
pub struct SessionBudget {
    remaining: u32,
}

impl SessionBudget {
    /// Snapshot today's remaining VOCABULARY new-card allowance (cap minus
    /// non-letters cards first reviewed today, saturating at 0).
    pub fn open(db: &Database) -> Self {
        Self {
            remaining: new_cards_remaining_today(db),
        }
    }

    /// Allowance left in this snapshot.
    pub fn remaining(&self) -> u32 {
        self.remaining
    }

    /// Admit one more new card into the session plan; returns false (and
    /// admits nothing) once the snapshot is exhausted.
    pub fn try_admit(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }

    /// Track-aware admission: letters never consume the vocabulary snapshot;
    /// every other track uses the ordinary consuming operation above.
    pub fn try_admit_for_textbook(&mut self, textbook: &str) -> bool {
        textbook == "letters" || self.try_admit()
    }
}

/// Measured-assessment totals over `review_logs` rows written by the
/// practice flow (`source = 'practice'`): returns (total, correct) where a
/// result counts as correct iff its rating is >= 3. Returns (0, 0) on error.
pub fn practice_stats(db: &Database) -> (u32, u32) {
    db.conn()
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN rating >= 3 THEN 1 ELSE 0 END), 0)
             FROM review_logs
             WHERE source = 'practice'",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map(|(total, correct)| (total as u32, correct as u32))
        .unwrap_or((0, 0))
}

/// Load the stored memory state for `card_id`. `Ok(None)` iff the card has
/// never been reviewed; genuine SQL/decode errors (e.g. a non-numeric
/// `difficulty` in a REAL column) are REAL errors propagated to the caller
/// so a corrupt row is never mistaken for "no state" and silently replaced.
fn load_memory_state(db: &Database, card_id: &str) -> anyhow::Result<Option<MemoryStateRow>> {
    match db.conn().query_row(
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
    ) {
        Ok(state) => Ok(Some(state)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
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
pub(crate) fn days_between(from: &str, to: &str) -> u32 {
    let from_days = iso_to_days(from).unwrap_or(0);
    let to_days = iso_to_days(to).unwrap_or(0);
    to_days.saturating_sub(from_days) as u32
}

/// Parse the `YYYY-MM-DD` date prefix of an ISO 8601 timestamp into
/// (year, month, day). Boundary-safe: uses indexed byte access only
/// (never raw slicing, so multibyte UTF-8 in the first 10 bytes cannot
/// panic). Validates ASCII digits, `-` separators, month 1..=12, day
/// 1..=31, and rejects year 0. Anything after byte 9 (the time part)
/// is ignored. `pub(crate)` so other modules (e.g. sync ingress) can
/// validate date strings with the same rules.
pub(crate) fn parse_iso_ymd(iso: &str) -> Option<(u64, u64, u64)> {
    let b = iso.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let digit = |i: usize| -> Option<u64> {
        let c = *b.get(i)?;
        if c.is_ascii_digit() {
            Some((c - b'0') as u64)
        } else {
            None
        }
    };
    if *b.get(4)? != b'-' || *b.get(7)? != b'-' {
        return None;
    }
    let year = digit(0)? * 1000 + digit(1)? * 100 + digit(2)? * 10 + digit(3)?;
    let month = digit(5)? * 10 + digit(6)?;
    let day = digit(8)? * 10 + digit(9)?;
    if year == 0 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// Parse ISO 8601 date to days since epoch (approximate, for scheduling).
/// Returns `None` for malformed input instead of panicking.
fn iso_to_days(iso: &str) -> Option<u64> {
    let (year, month, day) = parse_iso_ymd(iso)?;
    ymd_to_days(year, month, day)
}

/// Parse an ISO 8601 timestamp to seconds since the Unix epoch.
/// The time component is optional: legacy midnight rows like
/// "2026-07-19T00:00:00Z" parse naturally, and a missing or truncated time
/// defaults to 00:00:00 UTC so date-valid input never fails to parse.
pub(crate) fn iso_to_secs(iso: &str) -> Option<u64> {
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

/// Convert (year, month, day) to days since Unix epoch.
/// Returns `None` instead of underflow-panicking for year 0 with
/// month <= 2 (the `year - 1` adjustment) and for pre-epoch dates
/// whose day count would go negative. Callers normally pass values
/// already validated by `parse_iso_ymd`.
fn ymd_to_days(year: u64, month: u64, day: u64) -> Option<u64> {
    let y = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe).checked_sub(719468)
}

/// Add an FSRS interval (days, possibly fractional) to `now` and return a
/// full "YYYY-MM-DDTHH:MM:SSZ" timestamp. The interval is converted to
/// seconds (`days * 86400`, rounded) and added to the current instant —
/// deliberately with NO whole-day rounding, NO one-day minimum, and NO
/// midnight stamping, so sub-day learning steps (an "Again" card reappearing
/// minutes or hours later) are preserved. The result stays lexicographically
/// comparable with legacy midnight-stamped rows like "2026-07-19T00:00:00Z".
pub(crate) fn add_days_to_now(now: &str, days: f32) -> String {
    // `now` normally comes from the injected Clock / a canonical sync event
    // and always parses. A malformed input CLAMPS to the Unix epoch
    // (deterministic) rather than silently falling back to the wall clock:
    // an epoch-broken stored date must never "succeed" into moving target —
    // the sync fold replays this and wall-clock fallback would make two
    // devices diverge. Epoch-clamped results are immediately due (safe:
    // the card resurfaces instead of being lost).
    let now_secs = iso_to_secs(now).unwrap_or(0);
    let interval_secs = (days as f64 * 86400.0).round() as i64;
    let new_secs = (now_secs as i64).saturating_add(interval_secs).max(0) as u64;
    secs_to_iso(new_secs)
}

/// Add whole seconds to a timestamp (deterministic epoch-clamp like
/// [`add_days_to_now`]). Used for the learning-step ladder.
pub(crate) fn add_secs_to(ts: &str, secs: u64) -> String {
    secs_to_iso(iso_to_secs(ts).unwrap_or(0).saturating_add(secs))
}

/// History-derived learning signal (C6): a card is in (re)learning iff every
/// review so far was Again — i.e. `reps == lapses` — which trivially covers
/// brand-new cards (0 == 0). ANY passing rating graduates the card.
/// Both scheduling paths (incremental submit + sync fold) track reps/lapses
/// the same way, so this replays identically.
pub(crate) fn is_learning(prior_reps: i32, prior_lapses: i32) -> bool {
    prior_reps == prior_lapses
}

/// A wrong measured-practice answer on an already graduated card records a
/// lapse and triggers relearning but does not collapse its FSRS memory.
/// Shared by incremental submission and sync fold so source-aware replay is
/// exact. In-learning cards and flashcard self-ratings still use FSRS Again.
pub(crate) fn preserve_memory_for_practice_miss(
    source: &str,
    rating: u8,
    prior_reps: i32,
    prior_lapses: i32,
) -> bool {
    source == "practice" && rating == 1 && prior_reps > prior_lapses
}

/// PURE due-date decision for one review (C6): given only history-derived
/// inputs — the card's rep/lapse counts BEFORE this review, the rating, the
/// FSRS next-state interval, and the review's own timestamp — return the due
/// timestamp. NO wall clock, NO database reads: the incremental submit path
/// and the sync fold (`sync::rebuild_memory_states`) call this with their
/// respective contexts and MUST produce identical output for identical
/// event histories.
///
/// While the card is in (re)learning (see [`is_learning`]), the FSRS `due`
/// is overridden by the step ladder (docs/guide.html:109-114):
/// Again→1min, Hard→5min, Good→10min, all as durations from `review_ts`.
/// Every Again, including a graduated-card lapse, uses the one-minute step.
/// Easy — and passing ratings once the card has graduated — use FSRS
/// intervals unmodified.
pub(crate) fn step_due(
    prior_reps: i32,
    prior_lapses: i32,
    rating: u8,
    fsrs_interval_days: f32,
    review_ts: &str,
) -> String {
    // P2.14: Again on ANY card returns in one minute. The caller still
    // stores the Again memory from FSRS `next_states`; only its due interval
    // is replaced. Sync replay calls this same pure function.
    if rating == 1 {
        return add_secs_to(review_ts, STEP_AGAIN_SECS);
    }
    if is_learning(prior_reps, prior_lapses) {
        let step = match rating {
            1 => Some(STEP_AGAIN_SECS),
            2 => Some(STEP_HARD_SECS),
            3 => Some(STEP_GOOD_SECS),
            // 4 (Easy) and invalid ratings graduate into FSRS scheduling.
            _ => None,
        };
        if let Some(secs) = step {
            return add_secs_to(review_ts, secs);
        }
    }
    add_days_to_now(review_ts, fsrs_interval_days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };

    /// Manually-advanceable clock for pinning "now" in scheduling tests (C6).
    #[derive(Clone)]
    struct ManualClock(Arc<AtomicU64>);

    impl ManualClock {
        fn new(secs: u64) -> Self {
            Self(Arc::new(AtomicU64::new(secs)))
        }
        fn advance(&self, delta: u64) {
            self.0.fetch_add(delta, Ordering::SeqCst);
        }
        fn secs(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> String {
            secs_to_iso(self.secs())
        }
    }

    fn engine_with_clock(clock: &ManualClock) -> SrsEngine {
        SrsEngine::with_clock(Box::new(clock.clone()))
    }

    /// Fixed point in time used by the clock-driven tests:
    /// 2026-06-01T12:00:00Z (midday, so nothing accidentally hits midnight).
    fn fixed_now() -> u64 {
        iso_to_secs("2026-06-01T12:00:00Z").unwrap()
    }

    fn due_date_of(db: &Database, card_id: &str) -> String {
        db.conn()
            .query_row(
                "SELECT due_date FROM memory_states WHERE card_id = ?1",
                [card_id],
                |row| row.get(0),
            )
            .unwrap()
    }

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

    /// C1: the memory-state upsert, the review-log insert, and the HLC
    /// advance are one transaction — a fault on the log insert must roll
    /// back EVERYTHING, and the retry must land exactly once.
    #[test]
    fn test_submit_review_is_atomic_when_log_insert_fails() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // Reject every review_logs insert with a trigger (simulates a fault
        // after the memory-state upsert in the old autocommit implementation).
        db.conn()
            .execute_batch(
                "CREATE TRIGGER reject_review_log
                 BEFORE INSERT ON review_logs
                 BEGIN
                     SELECT RAISE(FAIL, 'boom');
                 END;",
            )
            .unwrap();

        let result = engine.submit_review(&db, "atomic-card", 3);
        assert!(
            result.is_err(),
            "submit_review must fail when the review_logs insert fails"
        );

        // Rollback must leave the database completely untouched:
        // no memory state ...
        let state_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM memory_states WHERE card_id = 'atomic-card'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state_count, 0, "memory_states upsert must roll back");
        // ... no review log ...
        let log_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM review_logs WHERE card_id = 'atomic-card'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(log_count, 0, "review_logs insert must roll back");
        // ... and the HLC setting must not have advanced.
        let hlc_written: Option<String> = db
            .conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = 'hlc_last_ms'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert!(
            hlc_written.is_none(),
            "HLC advance must roll back with the review"
        );

        // With the fault gone, the retry must land exactly once:
        // one repetition, one log row (no double-advance, no orphan).
        db.conn()
            .execute_batch("DROP TRIGGER reject_review_log;")
            .unwrap();
        engine.submit_review(&db, "atomic-card", 3).unwrap();

        let reps: i32 = db
            .conn()
            .query_row(
                "SELECT reps FROM memory_states WHERE card_id = 'atomic-card'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reps, 1, "retry must produce exactly one repetition");
        let log_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM review_logs WHERE card_id = 'atomic-card'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(log_count, 1, "retry must produce exactly one log row");
    }

    /// C8: a memory_states row that cannot be decoded (non-numeric
    /// `difficulty` in a REAL column) is a genuine error, NOT "no state" —
    /// the review must fail and the row's history must survive untouched.
    #[test]
    fn test_submit_review_errors_on_corrupt_memory_state() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // SQLite's flexible typing lets TEXT land in a REAL column.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, last_review, due_date, reps, lapses)
                 VALUES ('corrupt-card', 12.5, 'not-a-number', '2020-01-01T00:00:00Z', '2020-01-02T00:00:00Z', 8, 2)",
                [],
            )
            .unwrap();

        let result = engine.submit_review(&db, "corrupt-card", 3);
        assert!(
            result.is_err(),
            "a corrupt stored state must error, not be treated as 'no state'"
        );

        // The row must be left untouched (reps 8 / lapses 2 preserved), not
        // silently replaced by a fresh "first review" state.
        let (reps, lapses, stability): (i32, i32, f32) = db
            .conn()
            .query_row(
                "SELECT reps, lapses, stability FROM memory_states WHERE card_id = 'corrupt-card'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (reps, lapses, stability),
            (8, 2, 12.5),
            "existing history must not be clobbered"
        );
        // And no phantom "first review" log may have been written.
        let log_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM review_logs WHERE card_id = 'corrupt-card'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(log_count, 0);
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
            ymd_to_days(2026, 7, 19).unwrap() * 86400
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
            .expect("preview query must not error on an unknown card")
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
        assert_eq!(new_cards_remaining_today(&db), DEFAULT_DAILY_NEW_CAP - 2);

        // Letters are still logged, but never count against the vocabulary
        // counter or its remaining allowance.
        insert_log("letters-001", &format!("{today}T13:00:00Z"));
        assert_eq!(new_cards_today(&db), 2);
        assert_eq!(new_cards_remaining_today(&db), DEFAULT_DAILY_NEW_CAP - 2);
    }

    /// C3: the session budget consults the daily allowance ONCE and hands
    /// admissions out of that snapshot; real submissions (not plans) are
    /// what later snapshots see deducted.
    #[test]
    fn test_session_budget_single_consultation() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // Fresh db: the full cap, admitted exactly once per card.
        let mut budget = SessionBudget::open(&db);
        assert_eq!(budget.remaining(), DEFAULT_DAILY_NEW_CAP);
        for _ in 0..DEFAULT_DAILY_NEW_CAP {
            assert!(budget.try_admit());
        }
        assert!(
            !budget.try_admit(),
            "the snapshot must be exhausted at the cap"
        );
        assert_eq!(budget.remaining(), 0);
        assert!(budget.try_admit_for_textbook("letters"));
        assert!(budget.try_admit_for_textbook("letters"));
        assert_eq!(budget.remaining(), 0, "letters consume no vocabulary slots");

        // Planning admits nothing durable: only successful submissions move
        // the counter a LATER snapshot consults.
        engine.submit_review(&db, "spent-1", 3).unwrap();
        engine.submit_review(&db, "spent-2", 3).unwrap();
        let later = SessionBudget::open(&db);
        assert_eq!(later.remaining(), DEFAULT_DAILY_NEW_CAP - 2);
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
        let clock = ManualClock::new(fixed_now());
        let engine = engine_with_clock(&clock);

        let before_secs = clock.secs();
        engine.submit_review(&db, "again-card", 1).unwrap();

        let due_date = due_date_of(&db, "again-card");
        let due_secs = iso_to_secs(&due_date).expect("due_date must be a full ISO timestamp");

        // C6 step ladder (docs/guide.html:109-114): a learning card rated
        // Again comes back in EXACTLY 1 minute — not FSRS's ~9h40m
        // retention interval, and not the old midnight stamp.
        assert_eq!(
            due_secs - before_secs,
            STEP_AGAIN_SECS,
            "new card rated Again must be due one learning step (60s) later"
        );
        assert!(
            !due_date.ends_with("T00:00:00Z"),
            "due_date must not be midnight-stamped: {due_date}"
        );
    }

    /// C6: a first-failure card resurfaces in the SAME session once its
    /// 1-minute step elapses.
    #[test]
    fn test_new_card_again_due_in_one_minute_and_resurfaces() {
        let db = Database::open(":memory:", None).unwrap();
        let clock = ManualClock::new(fixed_now());
        let engine = engine_with_clock(&clock);

        engine.submit_review(&db, "ladder-card", 1).unwrap();
        assert_eq!(
            due_date_of(&db, "ladder-card"),
            secs_to_iso(fixed_now() + STEP_AGAIN_SECS)
        );

        // Immediately after rating: not yet due.
        assert!(
            engine
                .get_due_cards(&db, 10)
                .unwrap()
                .iter()
                .all(|c| c.card.id != "ladder-card"),
            "card must not be due before its step elapses"
        );

        // One second past the 60s step: it comes back in this session.
        clock.advance(STEP_AGAIN_SECS + 1);
        let due = engine.get_due_cards(&db, 10).unwrap();
        assert!(
            due.iter().any(|c| c.card.id == "ladder-card"),
            "card must resurface once the Again step (60s) has elapsed"
        );
    }

    /// C6: the card is still in learning after a first Again (reps == lapses),
    /// so a second consecutive Again reuses the ladder rather than jumping
    /// to FSRS's multi-hour retention interval.
    #[test]
    fn test_second_consecutive_again_reuses_step_ladder() {
        let db = Database::open(":memory:", None).unwrap();
        let clock = ManualClock::new(fixed_now());
        let engine = engine_with_clock(&clock);

        engine.submit_review(&db, "ladder-card-2", 1).unwrap();
        clock.advance(STEP_AGAIN_SECS + 1);
        engine.submit_review(&db, "ladder-card-2", 1).unwrap();

        assert_eq!(
            due_date_of(&db, "ladder-card-2"),
            secs_to_iso(clock.secs() + STEP_AGAIN_SECS),
            "second consecutive Again must reuse the 60s step"
        );
        let (reps, lapses): (i32, i32) = db
            .conn()
            .query_row(
                "SELECT reps, lapses FROM memory_states WHERE card_id = 'ladder-card-2'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((reps, lapses), (2, 2));
    }

    /// C6: once a card graduates (a passing rating leaves reps > lapses),
    /// later reviews schedule via FSRS — clearly beyond the ladder's
    /// 10-minute maximum.
    #[test]
    fn test_card_past_learning_steps_schedules_via_fsrs() {
        let db = Database::open(":memory:", None).unwrap();
        let clock = ManualClock::new(fixed_now());
        let engine = engine_with_clock(&clock);

        // Again (1min step) -> Good (10min step, graduates) -> Good (FSRS)
        engine.submit_review(&db, "grad-card", 1).unwrap();
        clock.advance(STEP_AGAIN_SECS + 1);
        engine.submit_review(&db, "grad-card", 3).unwrap();
        assert_eq!(
            due_date_of(&db, "grad-card"),
            secs_to_iso(clock.secs() + STEP_GOOD_SECS),
            "Good on a learning card must take the 10-minute step"
        );

        clock.advance(STEP_GOOD_SECS + 1);
        engine.submit_review(&db, "grad-card", 3).unwrap();
        let gap = iso_to_secs(&due_date_of(&db, "grad-card")).unwrap() - clock.secs();
        assert!(
            gap > 2 * STEP_GOOD_SECS,
            "graduated card must schedule via FSRS, not the ladder: gap {gap}s"
        );
    }

    /// P2.14: a lapse on a graduated card reuses the one-minute due step,
    /// not the mature FSRS retention interval. Its stability/difficulty are
    /// still FSRS's Again next-state and full sync replay is byte-exact.
    #[test]
    fn test_graduated_again_relearns_in_one_minute_and_replays_exactly() {
        let db = Database::open(":memory:", None).unwrap();
        let clock = ManualClock::new(fixed_now());
        let engine = engine_with_clock(&clock);
        engine.submit_review(&db, "mature-lapse", 4).unwrap();
        clock.advance(86400);
        engine.submit_review(&db, "mature-lapse", 3).unwrap();
        let stability_before: f32 = db
            .conn()
            .query_row(
                "SELECT stability FROM memory_states WHERE card_id = 'mature-lapse'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        clock.advance(86400);
        engine.submit_review(&db, "mature-lapse", 1).unwrap();
        assert_eq!(
            due_date_of(&db, "mature-lapse"),
            secs_to_iso(clock.secs() + STEP_AGAIN_SECS)
        );
        assert_eq!(
            step_due(8, 2, 1, 30.0, &clock.now()),
            secs_to_iso(clock.secs() + STEP_AGAIN_SECS),
            "graduated Again must not copy the mature FSRS interval"
        );
        assert_eq!(
            engine
                .preview_intervals(&db, "mature-lapse")
                .unwrap()
                .unwrap()
                .again_secs,
            STEP_AGAIN_SECS as u32
        );
        let stability_after: f32 = db
            .conn()
            .query_row(
                "SELECT stability FROM memory_states WHERE card_id = 'mature-lapse'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stability_after > 0.0 && stability_after != stability_before);

        let expected: (f32, f32, Option<String>, String, i32, i32) = db
            .conn()
            .query_row(
                "SELECT stability, difficulty, last_review, due_date, reps, lapses
                 FROM memory_states WHERE card_id = 'mature-lapse'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        let events = crate::sync::events_since(&db, None).unwrap();
        let replica = Database::open(":memory:", None).unwrap();
        crate::sync::apply_review_events(&replica, &events).unwrap();
        let replayed: (f32, f32, Option<String>, String, i32, i32) = replica
            .conn()
            .query_row(
                "SELECT stability, difficulty, last_review, due_date, reps, lapses
                 FROM memory_states WHERE card_id = 'mature-lapse'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(replayed, expected, "incremental lapse == sync fold");

        // The next passing preview is FSRS again (not a copied long Again
        // interval), while every Again preview stays one minute.
        assert!(
            engine
                .preview_intervals(&db, "mature-lapse")
                .unwrap()
                .unwrap()
                .good_secs
                > STEP_GOOD_SECS as u32
        );
    }

    #[test]
    fn test_graduated_practice_miss_preserves_memory_but_relearns_in_one_minute() {
        let db = Database::open(":memory:", None).unwrap();
        let clock = ManualClock::new(fixed_now());
        let engine = engine_with_clock(&clock);
        engine.submit_review(&db, "practice-lapse", 3).unwrap();
        let before: (f32, f32, i32, i32) = db
            .conn()
            .query_row(
                "SELECT stability, difficulty, reps, lapses
                 FROM memory_states WHERE card_id = 'practice-lapse'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert!(before.2 > before.3, "fixture must be graduated");

        clock.advance(86400);
        engine
            .submit_review_with_source(&db, "practice-lapse", 1, "practice")
            .unwrap();
        let after: (f32, f32, i32, i32) = db
            .conn()
            .query_row(
                "SELECT stability, difficulty, reps, lapses
                 FROM memory_states WHERE card_id = 'practice-lapse'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!((after.0, after.1), (before.0, before.1));
        assert_eq!((after.2, after.3), (before.2 + 1, before.3 + 1));
        assert_eq!(
            due_date_of(&db, "practice-lapse"),
            secs_to_iso(clock.secs() + STEP_AGAIN_SECS)
        );
        assert_eq!(
            engine
                .preview_intervals(&db, "practice-lapse")
                .unwrap()
                .unwrap()
                .again_secs,
            STEP_AGAIN_SECS as u32
        );

        // In-learning practice misses still select FSRS Again normally.
        engine
            .submit_review_with_source(&db, "learning-practice", 1, "practice")
            .unwrap();
        let learning_before: (f32, f32) = db
            .conn()
            .query_row(
                "SELECT stability, difficulty FROM memory_states
                 WHERE card_id = 'learning-practice'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        clock.advance(STEP_AGAIN_SECS + 1);
        engine
            .submit_review_with_source(&db, "learning-practice", 1, "practice")
            .unwrap();
        let learning_after: (f32, f32) = db
            .conn()
            .query_row(
                "SELECT stability, difficulty FROM memory_states
                 WHERE card_id = 'learning-practice'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_ne!(learning_after, learning_before);
    }

    /// C6 defense-in-depth: a malformed/epoch-broken stored timestamp must
    /// NOT silently succeed against the wall clock (that fallback made the
    /// sync fold device-dependent). It now clamps deterministically to the
    /// Unix epoch — the card lands far in the past, i.e. immediately due,
    /// never lost.
    #[test]
    fn test_add_days_to_now_malformed_clamps_to_epoch() {
        // Garbage input: epoch + 1 day, no wall-clock involvement.
        assert_eq!(add_days_to_now("not a date", 1.0), "1970-01-02T00:00:00Z");
        // Epoch-broken stored date (year 0 is ASCII-valid but unparseable):
        // deterministic clamp, and demonstrably not "today".
        let clamped = add_days_to_now("0000-01-01T00:00:00Z", 0.0);
        assert_eq!(clamped, "1970-01-01T00:00:00Z");
        let wall_now = iso_to_secs(&chrono_now()).unwrap();
        assert!(
            wall_now - iso_to_secs(&clamped).unwrap() > 30 * 86400,
            "malformed input must not silently succeed near the wall clock"
        );
    }

    #[test]
    fn test_submit_review_with_source_writes_source_column() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        // The legacy entry point tags rows 'flashcard'...
        engine.submit_review(&db, "card-flash", 3).unwrap();
        // ...and the explicit-source variant records what it is given.
        engine
            .submit_review_with_source(&db, "card-practice", 3, "practice")
            .unwrap();

        let source_of = |card_id: &str| -> String {
            db.conn()
                .query_row(
                    "SELECT source FROM review_logs WHERE card_id = ?1",
                    [card_id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(source_of("card-flash"), "flashcard");
        assert_eq!(source_of("card-practice"), "practice");

        // Practice submissions still drive the same scheduling machinery.
        let reps: i32 = db
            .conn()
            .query_row(
                "SELECT reps FROM memory_states WHERE card_id = 'card-practice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reps, 1);
    }

    #[test]
    fn test_practice_stats_counts_only_practice_rows() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();

        assert_eq!(practice_stats(&db), (0, 0));

        engine
            .submit_review_with_source(&db, "p-wrong", 1, "practice")
            .unwrap();
        engine
            .submit_review_with_source(&db, "p-good", 3, "practice")
            .unwrap();
        engine
            .submit_review_with_source(&db, "p-easy", 4, "practice")
            .unwrap();
        // Flashcard reviews, even Good/Easy ones, must not be counted.
        engine.submit_review(&db, "f-good", 4).unwrap();

        assert_eq!(
            practice_stats(&db),
            (3, 2),
            "correct := rating >= 3 over source='practice' rows only"
        );
    }

    #[test]
    fn test_days_to_ymd_and_back() {
        // 1970-01-01 = day 0
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        // Round-trip
        assert_eq!(ymd_to_days(1970, 1, 1), Some(0));
        assert_eq!(ymd_to_days(2024, 6, 15), iso_to_days("2024-06-15"));
    }

    #[test]
    fn test_iso_to_days() {
        let d1 = iso_to_days("2024-01-01T00:00:00Z").unwrap();
        let d2 = iso_to_days("2024-01-02T00:00:00Z").unwrap();
        assert_eq!(d2 - d1, 1);
    }

    #[test]
    fn test_parse_iso_ymd_valid() {
        assert_eq!(parse_iso_ymd("2024-06-15T12:00:00Z"), Some((2024, 6, 15)));
        // Date-only input (no time part) parses too.
        assert_eq!(parse_iso_ymd("1970-01-01"), Some((1970, 1, 1)));
    }

    #[test]
    fn test_parse_iso_ymd_rejects_malformed() {
        // Multibyte UTF-8 in the first 10 bytes (used to panic on raw slicing)
        assert_eq!(parse_iso_ymd("202α-10-09T12:00:00Z"), None);
        // Year 0
        assert_eq!(parse_iso_ymd("0000-01-01"), None);
        // Bad month/day ranges
        assert_eq!(parse_iso_ymd("2024-00-10"), None);
        assert_eq!(parse_iso_ymd("2024-13-01"), None);
        assert_eq!(parse_iso_ymd("2024-01-00"), None);
        assert_eq!(parse_iso_ymd("2024-01-32"), None);
        // Truncated
        assert_eq!(parse_iso_ymd("2024-01"), None);
        assert_eq!(parse_iso_ymd(""), None);
        assert_eq!(parse_iso_ymd("202-01-01"), None);
        // Non-digit where a digit belongs
        assert_eq!(parse_iso_ymd("202a-01-01"), None);
        // Missing separators
        assert_eq!(parse_iso_ymd("202401011"), None);
        assert_eq!(parse_iso_ymd("2024/01/01"), None);
    }

    #[test]
    fn test_iso_to_days_and_secs_reject_malformed() {
        assert_eq!(iso_to_days("202α-10-09T12:00:00Z"), None);
        assert_eq!(iso_to_days("0000-01-01"), None);
        assert_eq!(iso_to_days("2024-13-40"), None);
        assert_eq!(iso_to_days("2024-06"), None);
        assert_eq!(iso_to_secs("202α-10-09T12:00:00Z"), None);
        assert_eq!(iso_to_secs("0000-01-01T00:00:00Z"), None);
        assert_eq!(iso_to_secs("2024-13-40T99:99:99Z"), None);
    }

    #[test]
    fn test_ymd_to_days_rejects_underflow() {
        // Year 0 with month <= 2 would underflow `year - 1`
        assert_eq!(ymd_to_days(0, 1, 1), None);
        assert_eq!(ymd_to_days(0, 2, 28), None);
        // Pre-epoch dates underflow the Unix-epoch subtraction
        assert_eq!(ymd_to_days(1, 1, 1), None);
        // Valid dates still work
        assert_eq!(ymd_to_days(1970, 1, 1), Some(0));
    }

    #[test]
    fn test_days_between_malformed_never_panics() {
        use std::panic::catch_unwind;
        let bad = [
            "202α-10-09T12:00:00Z",
            "0000-01-01",
            "2024-13-40",
            "2024-00-00",
            "2024-06",
            "not a date",
            "",
        ];
        // Fold over every ordered pair through days_between, proving the
        // whole path (parse -> ymd_to_days -> days_between) is panic-free.
        let result = catch_unwind(|| {
            bad.iter().fold(0u32, |acc, from| {
                bad.iter()
                    .fold(acc, |acc2, to| acc2.saturating_add(days_between(from, to)))
            })
        });
        assert!(result.is_ok(), "days_between must never panic on bad input");
    }
}
