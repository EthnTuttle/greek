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

    /// Get cards that are due for review (due_date <= now), ordered by most overdue first
    pub fn get_due_cards(&self, db: &Database, limit: usize) -> Vec<CardWithState> {
        let now = chrono_now();
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT card_id, stability, difficulty, last_review, due_date, reps, lapses
                 FROM memory_states
                 WHERE due_date <= ?1
                 ORDER BY due_date ASC
                 LIMIT ?2",
            )
            .unwrap_or_else(|e| panic!("Failed to prepare due cards query: {e}"));

        let rows = stmt
            .query_map(rusqlite::params![now, limit as i64], |row| {
                Ok(MemoryStateRow {
                    card_id: row.get(0)?,
                    stability: row.get(1)?,
                    difficulty: row.get(2)?,
                    last_review: row.get(3)?,
                    due_date: row.get(4)?,
                    reps: row.get(5)?,
                    lapses: row.get(6)?,
                })
            })
            .unwrap_or_else(|e| panic!("Failed to query due cards: {e}"));

        rows.filter_map(|row| {
            let memory = row.ok()?;
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
            Some(CardWithState {
                card,
                memory: Some(memory),
            })
        })
        .collect()
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

    /// Submit a review and update the memory state
    pub fn submit_review(&self, db: &Database, card_id: &str, rating: u8) -> anyhow::Result<()> {
        let now = chrono_now();

        // Load current memory state
        let current_state: Option<MemoryStateRow> = db
            .conn()
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
            .ok();

        // Calculate elapsed days since last review
        let elapsed_days: u32 = if let Some(ref state) = current_state {
            if let Some(ref last_review) = state.last_review {
                days_between(last_review, &now)
            } else {
                0
            }
        } else {
            0
        };

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

/// Returns the current time as an ISO 8601 string (UTC).
/// Uses a simple approach without external chrono dependency.
fn chrono_now() -> String {
    // We format as ISO 8601 using the system clock
    // Since we don't have chrono, we use a simple approach
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    // Convert unix timestamp to ISO 8601
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

/// Add fractional days to an ISO date string, returning a new ISO date string
fn add_days_to_now(now: &str, days: f32) -> String {
    let current_days = iso_to_days(now).unwrap_or(0);
    let new_days = current_days + days.round().max(1.0) as u64;
    let (y, m, d) = days_to_ymd(new_days);
    format!("{:04}-{:02}-{:02}T00:00:00Z", y, m, d)
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
        let due = engine.get_due_cards(&db, 10);

        // Only the overdue card should be returned
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].card.id, "due-card");
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
