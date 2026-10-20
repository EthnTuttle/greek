//! The `share_outbox` sqlite table: one row per (slug, triggering-instance)
//! dedupe key, owning the **fully-signed kind-1 event JSON verbatim** so a
//! retry re-sends byte-identical events (idempotent for relays; design
//! §6 / Appendix R "Delivery correctness").
//!
//! State machine:
//!
//! ```text
//!   queued ──flush publish ok──▶ posted             (terminal; history kept)
//!   queued ──flush transient───▶ posting ──▶ queued (retry)
//!   queued ──enqueue/flush caps──▶ skipped_cap      (terminal; history kept)
//!   queued ──flush perma fail──▶ failed_perma       (terminal; history kept)
//!   queued/posting ──24h───────▶ failed_perma       (expiry sweep)
//! ```
//!
//! This crate deliberately does NOT touch `crates/core`'s migration system
//! (ownership boundary): [`init_schema`] is a `CREATE TABLE IF NOT EXISTS`
//! called lazily at the start of every operation that touches the table.

use nostr_sdk::prelude::Event;

use crate::errors::{Result, ShareError};

/// Rows in `queued`/`posting` that have gone unposted this long are expired
/// into `failed_perma` (Appendix R: 24h outbox expiry).
pub const OUTBOX_TTL_MS: i64 = 24 * 3600 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    Queued,
    Posting,
    Posted,
    SkippedCap,
    FailedPerma,
}

impl OutboxState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Posting => "posting",
            Self::Posted => "posted",
            Self::SkippedCap => "skipped_cap",
            Self::FailedPerma => "failed_perma",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        match s {
            "queued" => Ok(Self::Queued),
            "posting" => Ok(Self::Posting),
            "posted" => Ok(Self::Posted),
            "skipped_cap" => Ok(Self::SkippedCap),
            "failed_perma" => Ok(Self::FailedPerma),
            other => Err(ShareError::Db(rusqlite::Error::InvalidColumnName(format!(
                "unknown share_outbox state {other:?}"
            )))),
        }
    }

    /// Rows in these states still participate in flush/expiry logic.
    pub fn live(&self) -> bool {
        matches!(self, Self::Queued | Self::Posting)
    }
}

#[derive(Debug, Clone)]
pub struct OutboxRow {
    pub dedupe_key: String,
    pub slug: String,
    /// Fully-signed kind-1 event JSON (id/sig/created_at frozen).
    pub event_json: String,
    pub state: OutboxState,
    /// JSON map: relay URL → `"accepted"` | `"failed:<reason>"` | `"pending"`.
    pub per_relay: String,
    pub created_ms: i64,
    /// The triggering-instance id (milestone identity), for UI history.
    pub visible_id: String,
}

impl OutboxRow {
    /// Parse the stored event back to a nostr `Event`.
    pub fn event(&self) -> Result<Event> {
        serde_json::from_str(&self.event_json).map_err(ShareError::Json)
    }

    /// Parse the per-relay delivery map.
    pub fn per_relay_map(&self) -> std::collections::BTreeMap<String, String> {
        serde_json::from_str(&self.per_relay).unwrap_or_default()
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> std::result::Result<OutboxRow, rusqlite::Error> {
    let state_raw: String = row.get(3)?;
    let state = OutboxState::parse(&state_raw)
        .map_err(|_| rusqlite::Error::InvalidColumnName(state_raw.clone()))?;
    Ok(OutboxRow {
        dedupe_key: row.get(0)?,
        slug: row.get(1)?,
        event_json: row.get(2)?,
        state,
        per_relay: row.get(4)?,
        created_ms: row.get(5)?,
        visible_id: row.get(6)?,
    })
}

const SELECT_COLS: &str =
    "SELECT dedupe_key, slug, event_json, state, per_relay, created_ms, visible_id
     FROM share_outbox";

/// Idempotent schema creation — safe to call on every entry point.
pub fn init_schema(db: &greek_core::Database) -> Result<()> {
    db.conn().execute_batch(
        "CREATE TABLE IF NOT EXISTS share_outbox(
             dedupe_key TEXT PRIMARY KEY,
             slug TEXT NOT NULL,
             event_json TEXT NOT NULL,
             state TEXT NOT NULL DEFAULT 'queued',
             per_relay TEXT NOT NULL DEFAULT '{}',
             created_ms INTEGER NOT NULL,
             visible_id TEXT NOT NULL DEFAULT ''
         );",
    )?;
    Ok(())
}

/// Insert a new queued row carrying the fully-signed event bytes. Returns
/// `true` if the row was inserted, `false` if the dedupe key already exists
/// (idempotent — the original signed event, including its frozen
/// created_at/id, is kept as the canonical bytes).
pub fn enqueue(
    db: &greek_core::Database,
    dedupe_key: &str,
    slug: &str,
    event_json: &str,
    visible_id: &str,
    now_ms: i64,
) -> Result<bool> {
    init_schema(db)?;
    let n = db.conn().execute(
        "INSERT INTO share_outbox (dedupe_key, slug, event_json, state, per_relay, created_ms, visible_id)
         VALUES (?1, ?2, ?3, 'queued', '{}', ?4, ?5)
         ON CONFLICT(dedupe_key) DO NOTHING",
        rusqlite::params![dedupe_key, slug, event_json, now_ms, visible_id],
    )?;
    Ok(n == 1)
}

/// Insert a `skipped_cap` row directly (cap overflow at enqueue time — the
/// day is full, so the milestone is recorded as deliberately not posted).
/// Returns whether the row was inserted (dedupe-key idempotent).
pub fn enqueue_skipped_cap(
    db: &greek_core::Database,
    dedupe_key: &str,
    slug: &str,
    event_json: &str,
    visible_id: &str,
    now_ms: i64,
) -> Result<bool> {
    init_schema(db)?;
    let n = db.conn().execute(
        "INSERT INTO share_outbox (dedupe_key, slug, event_json, state, per_relay, created_ms, visible_id)
         VALUES (?1, ?2, ?3, 'skipped_cap', '{}', ?4, ?5)
         ON CONFLICT(dedupe_key) DO NOTHING",
        rusqlite::params![dedupe_key, slug, event_json, now_ms, visible_id],
    )?;
    Ok(n == 1)
}

/// Fetch one row by dedupe key.
pub fn get(db: &greek_core::Database, dedupe_key: &str) -> Result<Option<OutboxRow>> {
    init_schema(db)?;
    let mut stmt = db
        .conn()
        .prepare(&format!("{SELECT_COLS} WHERE dedupe_key = ?1"))?;
    let mut rows = stmt.query_map([dedupe_key], map_row)?;
    match rows.next() {
        Some(r) => Ok(Some(r?)),
        None => Ok(None),
    }
}

/// Live rows (`queued`/`posting`) oldest-first — the batch flush processes.
/// Call [`expire_stale`] first so over-TTL rows have already transitioned.
pub fn due_rows(db: &greek_core::Database) -> Result<Vec<OutboxRow>> {
    init_schema(db)?;
    let mut stmt = db.conn().prepare(&format!(
        "{SELECT_COLS} WHERE state IN ('queued', 'posting') ORDER BY created_ms ASC, dedupe_key ASC"
    ))?;
    let rows = stmt.query_map([], map_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// All rows newest-first — the per-slug post history surface.
pub fn history(db: &greek_core::Database) -> Result<Vec<OutboxRow>> {
    init_schema(db)?;
    let mut stmt = db
        .conn()
        .prepare(&format!("{SELECT_COLS} ORDER BY created_ms DESC, dedupe_key ASC"))?;
    let rows = stmt.query_map([], map_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Count of rows flush could still work on (`queued` only — `posting` rows
/// are re-published, they are not "pending to be attempted").
pub fn pending_count(db: &greek_core::Database) -> Result<u32> {
    init_schema(db)?;
    let n: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM share_outbox WHERE state = 'queued'",
        [],
        |row| row.get(0),
    )?;
    Ok(n as u32)
}

/// Transition a row's state; when `per_relay_json` is provided the delivery
/// map is replaced atomically with the state change.
pub fn set_state(
    db: &greek_core::Database,
    dedupe_key: &str,
    state: OutboxState,
    per_relay_json: Option<&str>,
) -> Result<()> {
    init_schema(db)?;
    match per_relay_json {
        Some(pr) => {
            db.conn().execute(
                "UPDATE share_outbox SET state = ?2, per_relay = ?3 WHERE dedupe_key = ?1",
                rusqlite::params![dedupe_key, state.as_str(), pr],
            )?;
        }
        None => {
            db.conn().execute(
                "UPDATE share_outbox SET state = ?2 WHERE dedupe_key = ?1",
                rusqlite::params![dedupe_key, state.as_str()],
            )?;
        }
    }
    Ok(())
}

/// Expire `queued`/`posting` rows older than `ttl_ms` (default policy:
/// [`OUTBOX_TTL_MS`]) into `failed_perma`, recording the expiry in the
/// per-relay map. `skipped_cap`/`failed_perma`/`posted` rows are historic
/// and never expire. Returns the number of rows transitioned.
pub fn expire_stale(db: &greek_core::Database, now_ms: i64) -> Result<u32> {
    init_schema(db)?;
    let cutoff = now_ms - OUTBOX_TTL_MS;
    let stale: Vec<String> = {
        let mut stmt = db.conn().prepare(
            "SELECT dedupe_key, per_relay FROM share_outbox
             WHERE state IN ('queued','posting') AND created_ms < ?1",
        )?;
        let rows = stmt.query_map([cutoff], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<(String, String)>, _>>()?
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    };
    for key in &stale {
        let row = get(db, key)?;
        if let Some(row) = row {
            let mut map = row.per_relay_map();
            map.insert("_meta".to_string(), "failed:expired after 24h".to_string());
            set_state(db, key, OutboxState::FailedPerma, Some(&serde_json::to_string(&map)?))?;
        }
    }
    Ok(stale.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use greek_core::Database;

    const NOW: i64 = 1_760_000_000_000;

    fn signed_event_json(content: &str) -> String {
        let keys = crate::identity::ShareKeys::derive_from_sync(&[0xAB; 32]).unwrap();
        serde_json::to_string(&keys.sign_note(content).unwrap()).unwrap()
    }

    #[test]
    fn enqueue_is_dedupe_key_idempotent_and_preserves_original_bytes() {
        let db = Database::open(":memory:", None).unwrap();
        let ev1 = signed_event_json("first");
        assert!(enqueue(&db, "k1", "streak.7", &ev1, "2026-10-19", NOW).unwrap());
        // Second enqueue with DIFFERENT bytes must be a no-op.
        let ev2 = signed_event_json("retry with different signed bytes");
        assert!(!enqueue(&db, "k1", "streak.7", &ev2, "2026-10-19", NOW + 1).unwrap());
        let row = get(&db, "k1").unwrap().unwrap();
        assert_eq!(row.event_json, ev1, "original signed bytes are canonical");
        assert_eq!(row.state, OutboxState::Queued);
        assert_eq!(row.created_ms, NOW);
        assert_eq!(row.slug, "streak.7");
        assert_eq!(row.visible_id, "2026-10-19");
    }

    #[test]
    fn stored_event_roundtrips_to_valid_nostr_event() {
        let db = Database::open(":memory:", None).unwrap();
        let ev = signed_event_json("canonical content");
        enqueue(&db, "k2", "chapter.mastered", &ev, "athenaze/3", NOW).unwrap();
        let parsed = get(&db, "k2").unwrap().unwrap().event().unwrap();
        assert_eq!(parsed.kind, nostr_sdk::prelude::Kind::TextNote);
        assert_eq!(parsed.content, "canonical content");
        assert!(parsed.verify().is_ok());
    }

    #[test]
    fn pending_count_tracks_queued_only() {
        let db = Database::open(":memory:", None).unwrap();
        assert_eq!(pending_count(&db).unwrap(), 0);
        let ev = signed_event_json("x");
        enqueue(&db, "a", "streak.7", &ev, "v", NOW).unwrap();
        enqueue(&db, "b", "streak.30", &ev, "v", NOW).unwrap();
        assert_eq!(pending_count(&db).unwrap(), 2);
        set_state(&db, "a", OutboxState::Posted, None).unwrap();
        assert_eq!(pending_count(&db).unwrap(), 1);
        set_state(&db, "b", OutboxState::SkippedCap, None).unwrap();
        assert_eq!(pending_count(&db).unwrap(), 0);
    }

    #[test]
    fn expiry_moves_old_live_rows_to_failed_perma_and_keeps_history() {
        let db = Database::open(":memory:", None).unwrap();
        let ev = signed_event_json("x");
        enqueue(&db, "old-queued", "streak.7", &ev, "v", NOW - OUTBOX_TTL_MS - 1).unwrap();
        enqueue(&db, "fresh-queued", "streak.30", &ev, "v", NOW).unwrap();
        enqueue(&db, "old-skipped", "streak.100", &ev, "v", NOW - OUTBOX_TTL_MS - 1).unwrap();
        set_state(&db, "old-skipped", OutboxState::SkippedCap, None).unwrap();
        set_state(&db, "old-queued", OutboxState::Posting, None).unwrap();

        assert_eq!(expire_stale(&db, NOW).unwrap(), 1);
        let old = get(&db, "old-queued").unwrap().unwrap();
        assert_eq!(old.state, OutboxState::FailedPerma);
        assert!(old.per_relay.contains("expired"));
        assert_eq!(get(&db, "fresh-queued").unwrap().unwrap().state, OutboxState::Queued);
        // Terminal states stay for history, untouched by expiry.
        assert_eq!(get(&db, "old-skipped").unwrap().unwrap().state, OutboxState::SkippedCap);
        // Idempotent second sweep.
        assert_eq!(expire_stale(&db, NOW).unwrap(), 0);
        // History keeps all three rows.
        assert_eq!(history(&db).unwrap().len(), 3);
    }

    #[test]
    fn due_rows_are_oldest_first_and_skip_terminal() {
        let db = Database::open(":memory:", None).unwrap();
        let ev = signed_event_json("x");
        enqueue(&db, "c", "streak.7", &ev, "v", NOW + 2).unwrap();
        enqueue(&db, "a", "streak.7", &ev, "v", NOW).unwrap();
        enqueue(&db, "b", "streak.7", &ev, "v", NOW + 1).unwrap();
        set_state(&db, "b", OutboxState::Posted, None).unwrap();
        let due = due_rows(&db).unwrap();
        let keys: Vec<&str> = due.iter().map(|r| r.dedupe_key.as_str()).collect();
        assert_eq!(keys, ["a", "c"]);
    }

    #[test]
    fn skipped_cap_rows_insert_and_are_terminal() {
        let db = Database::open(":memory:", None).unwrap();
        let ev = signed_event_json("x");
        assert!(enqueue_skipped_cap(&db, "cap", "streak.7", &ev, "v", NOW).unwrap());
        assert!(!enqueue_skipped_cap(&db, "cap", "streak.7", &ev, "v", NOW).unwrap());
        let row = get(&db, "cap").unwrap().unwrap();
        assert_eq!(row.state, OutboxState::SkippedCap);
        assert!(due_rows(&db).unwrap().is_empty());
        // Not expired even after the TTL: skipped_cap is historic.
        assert_eq!(expire_stale(&db, NOW + OUTBOX_TTL_MS + 1).unwrap(), 0);
        assert_eq!(get(&db, "cap").unwrap().unwrap().state, OutboxState::SkippedCap);
    }
}
