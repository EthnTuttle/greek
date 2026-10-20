//! Sync-ready merge layer: CRDT semantics, transport-agnostic.
//!
//! Every `review_logs` row is a globally-addressable event identified by a
//! random v4 `uuid`, stamped with the originating `device_id` and a hybrid
//! logical clock (`hlc`, see [`Hlc`]). Merging is uuid-dedup insert plus a
//! deterministic fold ([`rebuild_memory_states`]) that recomputes FSRS
//! `memory_states` by replaying the full per-card event history in total
//! order `(hlc, uuid)` — so any set of devices that exchange the same events
//! converge to byte-identical memory state, no matter the arrival order.
//!
//! Settings merge by last-writer-wins on the HLC ([`setting_put`] /
//! [`setting_merge`]); [`pair_device`] / [`note_last_sync`] track peers.

use anyhow::Result;
use fsrs::MemoryState;

use crate::db::Database;

/// Hybrid logical clock: physical millisecond wall time, a tie-breaking
/// counter, and the originating node's id. The `Display`/`parse` format
/// `"{ms:013}-{counter:04}-{node:8}"` is lexicographically sortable ASCII, so
/// plain string ordering in SQL equals logical ordering.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hlc {
    pub ms: i64,
    pub counter: u16,
    pub node: String,
}

impl std::fmt::Display for Hlc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:013}-{:04}-{}", self.ms, self.counter, self.node)
    }
}

impl std::str::FromStr for Hlc {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let mut parts = s.split('-');
        let ms: i64 = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("bad HLC ms: {s:?}"))?;
        let counter: u16 = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("bad HLC counter: {s:?}"))?;
        let node = parts
            .next()
            .filter(|p| !p.is_empty())
            .ok_or_else(|| anyhow::anyhow!("bad HLC node: {s:?}"))?
            .to_string();
        Ok(Hlc { ms, counter, node })
    }
}

/// First 8 hex chars of a device id (dashes stripped) — the HLC node tag.
/// `pub` so the v2→v3 migration in `db` stamps rows with the same node form.
pub fn hlc_node_of(device_id: &str) -> String {
    device_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(8)
        .collect()
}

/// This device's stable id, bootstrapped once into `user_settings.device_id`.
pub fn device_id(db: &Database) -> String {
    if let Ok(existing) = db.conn().query_row(
        "SELECT value FROM user_settings WHERE key = 'device_id'",
        [],
        |row| row.get::<_, String>(0),
    ) {
        return existing;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = db.conn().execute(
        "INSERT INTO user_settings (key, value) VALUES ('device_id', ?1)
         ON CONFLICT(key) DO NOTHING",
        [&id],
    );
    db.conn()
        .query_row(
            "SELECT value FROM user_settings WHERE key = 'device_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or(id)
}

/// Produce a strictly-monotonic HLC for this device. The last millisecond
/// and counter persist in `user_settings` (`hlc_last_ms` / `hlc_last_counter`)
/// so ordering survives restarts; direct upserts (NOT [`setting_put`]) to
/// avoid recursion through [`hlc_now`].
pub fn hlc_now(db: &Database) -> Hlc {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let read = |key: &str| -> Option<i64> {
        db.conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = ?1",
                [key],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| v.parse().ok())
    };
    let last_ms = read("hlc_last_ms").unwrap_or(0);
    let last_counter = read("hlc_last_counter").unwrap_or(0);

    let (ms, counter) = if ms_now > last_ms {
        (ms_now, 0u16)
    } else {
        // Same millisecond (or a clock move backwards): advance the counter.
        (last_ms, last_counter.saturating_add(1) as u16)
    };

    let write = |key: &str, value: i64| {
        let _ = db.conn().execute(
            "INSERT INTO user_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value.to_string()],
        );
    };
    write("hlc_last_ms", ms);
    write("hlc_last_counter", counter as i64);

    Hlc {
        ms,
        counter,
        node: hlc_node_of(&device_id(db)),
    }
}

/// One review event — the portable, mergeable projection of a
/// `review_logs` row. `timestamp` is RFC3339 UTC as stored by the app.
/// Serde shape is frozen by SYNC-WIRE.md §2.1 (shared with transports).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReviewEvent {
    pub uuid: String,
    pub card_id: String,
    pub timestamp: String,
    pub rating: u32,
    pub elapsed_days: f64,
    pub scheduled_days: f64,
    pub source: String,
    pub device_id: String,
    pub hlc: String,
}

/// A sync batch — v1 of the sync wire format (SYNC-WIRE.md §2.1), the
/// canonical type both transports (nostr spool, iroh-docs) and the
/// backup export/import path serialize.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SyncBatch {
    /// Wire format version; must be 1.
    pub v: u32,
    pub device_id: String,
    pub from_hlc: String,
    pub to_hlc: String,
    pub reviews: Vec<ReviewEvent>,
    /// (key, value, hlc) settings triples.
    pub settings: Vec<(String, String, String)>,
}

/// All review events strictly after `since_hlc` (None = everything),
/// ordered by `(hlc, uuid)` — a safe keyset cursor because one device's HLCs
/// are strictly increasing (rows are unique in `(hlc)` per node).
pub fn events_since(db: &Database, since_hlc: Option<&str>) -> Result<Vec<ReviewEvent>> {
    let (sql, params): (&str, Vec<String>) = match since_hlc {
        Some(since) => (
            "SELECT uuid, card_id, timestamp, rating, elapsed_days, scheduled_days, source, device_id, hlc
             FROM review_logs WHERE hlc IS NOT NULL AND hlc > ?1
             ORDER BY hlc, uuid",
            vec![since.to_string()],
        ),
        None => (
            "SELECT uuid, card_id, timestamp, rating, elapsed_days, scheduled_days, source, device_id, hlc
             FROM review_logs WHERE hlc IS NOT NULL
             ORDER BY hlc, uuid",
            Vec::new(),
        ),
    };
    let mut stmt = db.conn().prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
        Ok(ReviewEvent {
            uuid: row.get(0)?,
            card_id: row.get(1)?,
            timestamp: row.get(2)?,
            rating: row.get::<_, i64>(3)? as u32,
            elapsed_days: row.get(4)?,
            scheduled_days: row.get(5)?,
            source: row.get(6)?,
            device_id: row.get(7)?,
            hlc: row.get(8)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Settings rows `(key, value, hlc)` strictly after `since_hlc`.
pub fn settings_since(
    db: &Database,
    since_hlc: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    let (sql, params): (&str, Vec<String>) = match since_hlc {
        Some(since) => (
            "SELECT key, value, hlc FROM settings_sync WHERE hlc > ?1 ORDER BY hlc, key",
            vec![since.to_string()],
        ),
        None => (
            "SELECT key, value, hlc FROM settings_sync ORDER BY hlc, key",
            Vec::new(),
        ),
    };
    let mut stmt = db.conn().prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Merge remote review events: uuid-dedup INSERT (FSRS state is NOT
/// incrementally updated per event), then one deterministic fold via
/// [`rebuild_memory_states`]. Returns the number of newly inserted rows.
///
/// Events can arrive over untrusted transports (a relay stores ciphertext,
/// but a compromised peer could inject plaintext): DROP anything that is
/// not a plausible review — rating must be 1..=4 (FSRS grades), card_id
/// must match the deck-id charset, timestamps/HLC must parse — instead of
/// letting garbage poison the fold.
pub fn apply_review_events(db: &Database, events: &[ReviewEvent]) -> Result<u32> {
    let mut inserted = 0u32;
    for e in events {
        if !(1..=4).contains(&e.rating)
            || e.card_id.is_empty()
            || e.card_id.len() > 128
            || !e
                .card_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            || e.hlc.parse::<Hlc>().is_err()
        {
            continue; // malformed / foreign-injected event: skip silently
        }
        let n = db.conn().execute(
            "INSERT INTO review_logs (uuid, card_id, timestamp, rating, elapsed_days, scheduled_days, source, device_id, hlc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(uuid) DO NOTHING",
            rusqlite::params![
                e.uuid,
                e.card_id,
                e.timestamp,
                e.rating as i64,
                e.elapsed_days,
                e.scheduled_days,
                e.source,
                e.device_id,
                e.hlc,
            ],
        )?;
        inserted += n as u32;
    }
    if inserted > 0 {
        rebuild_memory_states(db)?;
    }
    Ok(inserted)
}

/// LWW-merge settings rows `(key, value, hlc)`; returns rows applied.
pub fn apply_settings(db: &Database, kv: &[(String, String, String)]) -> Result<u32> {
    let mut applied = 0u32;
    for (key, value, hlc) in kv {
        if setting_merge(db, key, value, hlc)? {
            applied += 1;
        }
    }
    Ok(applied)
}

/// THE FOLD: for each card with review history, replay all events in total
/// order `(hlc, uuid)` and recompute FSRS state EXACTLY as the incremental
/// submit path (`SrsEngine::submit_review_with_source`) would have:
/// whole-day elapsed gaps against the previous event's timestamp, default
/// desired retention 0.9, rating outside 1-4 treated as Good, due date =
/// event timestamp + interval. `source` is mirrored as recorded (today the
/// incremental path schedules 'practice' rows identically to flashcards —
/// the fold mirrors that). Writes the resulting memory_states rows; rows for
/// cards with NO review history are untouched. Idempotent.
pub fn rebuild_memory_states(db: &Database) -> Result<u32> {
    let fsrs = fsrs::FSRS::new(Some(&[]))
        .map_err(|e| anyhow::anyhow!("default FSRS parameters should be valid: {e}"))?;
    const DESIRED_RETENTION: f32 = 0.9;

    let card_ids: Vec<String> = {
        let mut stmt = db
            .conn()
            .prepare("SELECT DISTINCT card_id FROM review_logs")?;
        let v = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v
    };

    let mut rewritten = 0u32;
    for card_id in &card_ids {
        let events: Vec<(String, i64)> = {
            let mut stmt = db.conn().prepare(
                "SELECT timestamp, rating FROM review_logs
                 WHERE card_id = ?1
                 ORDER BY hlc, uuid, id",
            )?;
            let v = stmt
                .query_map([card_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            v
        };
        if events.is_empty() {
            continue;
        }

        let mut stability: f32 = 0.0;
        let mut difficulty: f32 = 0.0;
        let mut last_review: Option<String> = None;
        let mut due_date = String::new();
        let mut reps: i64 = 0;
        let mut lapses: i64 = 0;

        for (ts, rating) in &events {
            let elapsed_days = last_review
                .as_deref()
                .map(|prev| crate::srs::days_between(prev, ts))
                .unwrap_or(0);
            let memory = last_review
                .as_ref()
                .map(|_| MemoryState { stability, difficulty });
            let next = fsrs
                .next_states(memory, DESIRED_RETENTION, elapsed_days)
                .map_err(|e| anyhow::anyhow!("FSRS next_states failed during fold: {e}"))?;
            let chosen = match rating {
                1 => &next.again,
                2 => &next.hard,
                3 => &next.good,
                4 => &next.easy,
                _ => &next.good,
            };
            stability = chosen.memory.stability;
            difficulty = chosen.memory.difficulty;
            due_date = crate::srs::add_days_to_now(ts, chosen.interval);
            reps += 1;
            if *rating == 1 {
                lapses += 1;
            }
            last_review = Some(ts.clone());
        }

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
            rusqlite::params![card_id, stability, difficulty, last_review, due_date, reps, lapses],
        )?;
        rewritten += 1;
    }
    Ok(rewritten)
}

/// nostr secret material must NEVER enter the synced/plaintext settings
/// tables (DESIGN-progress-sharing.md, Appendix R "Secrets"). A write is
/// refused when the KEY matches the glob `*nsec*` (case-insensitive) or the
/// VALUE contains a bech32 `nsec1…` secret (pattern
/// `nsec1[02-9ac-hj-np-z]{58}`, matched anywhere). Hand-rolled so the guard
/// adds zero dependencies.
fn secret_blocklist_reason(key: &str, value: &str) -> Option<&'static str> {
    let key_lc = key.to_lowercase();
    if key_lc.contains("nsec") {
        return Some("key matches the '*nsec*' blocklist");
    }
    if value_contains_nsec_bech32(value) {
        return Some("value matches the nsec1 bech32 pattern");
    }
    None
}

/// `true` if `value` contains `nsec1` followed by 58 chars of the bech32
/// data charset `[02-9ac-hj-np-z]` (i.e. excludes `1`, `b`, `i`, `o`),
/// matching the pattern `nsec1[02-9ac-hj-np-z]{58}` anywhere in the value.
fn value_contains_nsec_bech32(value: &str) -> bool {
    const NSEC_LEN: usize = 5 + 58; // "nsec1" + 58 data chars
    let bytes = value.as_bytes();
    let mut i = 0;
    while i + NSEC_LEN <= bytes.len() {
        if &bytes[i..i + 5] == b"nsec1" {
            let all_bech32 = bytes[i + 5..i + NSEC_LEN].iter().all(|c| {
                matches!(c, b'0' | b'2'..=b'9' | b'a' | b'c'..=b'h' | b'j'..=b'n' | b'p'..=b'z')
            });
            if all_bech32 {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Local settings write: stamps `hlc_now` and upserts BOTH `settings_sync`
/// (the sync view) and `user_settings` (what local readers consume today).
///
/// REFUSES anything shaped like a nostr secret (see
/// [`secret_blocklist_reason`]) — such values belong in the `secrets`
/// table via [`secret_put`], never in a table that syncs or exports.
pub fn setting_put(db: &Database, key: &str, value: &str) -> Result<()> {
    if let Some(reason) = secret_blocklist_reason(key, value) {
        anyhow::bail!(
            "refusing to store {key:?} in synced settings ({reason}); \
             use the secrets table (secret_put) instead"
        );
    }
    let hlc = hlc_now(db).to_string();
    db.conn().execute(
        "INSERT INTO settings_sync (key, value, hlc) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, hlc = excluded.hlc",
        rusqlite::params![key, value, hlc],
    )?;
    db.conn().execute(
        "INSERT INTO user_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// LWW merge for an incoming `(key, value, hlc)`: a strictly-greater incoming
/// HLC wins; on a win BOTH tables are written. Returns whether it was applied.
///
/// Rows shaped like a nostr secret (see [`secret_blocklist_reason`]) are
/// DROPPED before any HLC comparison, so no remote batch or imported backup
/// can bypass the blocklist to re-plant a secret into the synced tables.
pub fn setting_merge(db: &Database, key: &str, value: &str, hlc: &str) -> Result<bool> {
    if let Some(reason) = secret_blocklist_reason(key, value) {
        eprintln!("sync: refusing to merge setting {key:?} ({reason}); dropped");
        return Ok(false);
    }
    let incoming: Hlc = hlc.parse()?;
    let existing: Option<String> = db
        .conn()
        .query_row(
            "SELECT hlc FROM settings_sync WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok();
    let wins = match existing {
        Some(e) => {
            let cur: Hlc = e.parse()?;
            incoming > cur
        }
        None => true,
    };
    if wins {
        db.conn().execute(
            "INSERT INTO settings_sync (key, value, hlc) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, hlc = excluded.hlc",
            rusqlite::params![key, value, hlc],
        )?;
        db.conn().execute(
            "INSERT INTO user_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
    }
    Ok(wins)
}

/// Register a peer device (upsert; preserves `last_sync_hlc` if re-paired).
pub fn pair_device(db: &Database, device_id: &str, label: &str) -> Result<()> {
    db.conn().execute(
        "INSERT INTO sync_devices (device_id, label, paired_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(device_id) DO UPDATE SET label = excluded.label",
        rusqlite::params![device_id, label, crate::srs::now_iso8601()],
    )?;
    Ok(())
}

/// Record the high-water mark synced with a peer (upserts an unlabeled row
/// if the device was never paired).
pub fn note_last_sync(db: &Database, device_id: &str, hlc: &str) -> Result<()> {
    db.conn().execute(
        "INSERT INTO sync_devices (device_id, label, paired_at, last_sync_hlc)
         VALUES (?1, '', ?2, ?3)
         ON CONFLICT(device_id) DO UPDATE SET last_sync_hlc = excluded.last_sync_hlc",
        rusqlite::params![device_id, crate::srs::now_iso8601(), hlc],
    )?;
    Ok(())
}

// --- Secrets vault (schema v4) ---
//
// The `secrets` table is NEVER read by `settings_since`, never written by
// the settings sync paths, and never serialized into backup payloads — it
// exists precisely so nostr secret keys have a home that cannot leak into
// sync or exports (DESIGN-progress-sharing.md, Appendix R "Secrets").

/// Read a secret. `None` when the key is absent.
pub fn secret_get(db: &Database, key: &str) -> Result<Option<String>> {
    let v = db
        .conn()
        .query_row("SELECT value FROM secrets WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .ok();
    Ok(v)
}

/// Store a secret (upsert). Device-local only — never synced, never
/// included in backup exports.
pub fn secret_put(db: &Database, key: &str, value: &str) -> Result<()> {
    db.conn().execute(
        "INSERT INTO secrets (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Remove a secret; no-op when absent.
pub fn secret_delete(db: &Database, key: &str) -> Result<()> {
    db.conn()
        .execute("DELETE FROM secrets WHERE key = ?1", [key])?;
    Ok(())
}

/// `true` when the secret exists (no value read-out).
pub fn secret_has(db: &Database, key: &str) -> bool {
    db.conn()
        .query_row("SELECT 1 FROM secrets WHERE key = ?1", [key], |_| Ok(()))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::srs::SrsEngine;
    use rusqlite::Connection;

    /// Snapshot of `memory_states`, ordered by card_id — compared row-for-row.
    fn memory_snapshot(db: &Database) -> Vec<(String, f64, f64, Option<String>, String, i64, i64)> {
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT card_id, stability, difficulty, last_review, due_date, reps, lapses
                 FROM memory_states ORDER BY card_id",
            )
            .unwrap();
        stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
    }

    fn review_uuid_set(db: &Database) -> Vec<String> {
        let mut stmt = db.conn().prepare("SELECT uuid FROM review_logs ORDER BY uuid").unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<String>, _>>()
            .unwrap()
    }

    /// Deterministic xorshift Fisher-Yates shuffle (no rand dependency).
    fn shuffle(mut v: Vec<ReviewEvent>, seed: u64) -> Vec<ReviewEvent> {
        let mut s = seed.max(1);
        for i in (1..v.len()).rev() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let j = (s % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
        v
    }

    /// Bump an HLC string's millisecond field by `delta` (keeps counter/node).
    fn bump_ms(hlc: &str, delta: i64) -> String {
        let h: Hlc = hlc.parse().unwrap();
        Hlc { ms: h.ms + delta, ..h }.to_string()
    }

    // 1. Migration: a v2-shaped db converges to v3 with backfilled
    // uuid/device_id/hlc on pre-existing rows.
    #[test]
    fn test_migration_v2_to_v3_backfills_review_rows() {
        let dir = std::env::temp_dir().join(format!("greek-v2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v2.db");
        let path_str = path.to_string_lossy().into_owned();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE review_logs(
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     card_id TEXT NOT NULL,
                     timestamp TEXT NOT NULL,
                     rating INTEGER NOT NULL,
                     elapsed_days REAL NOT NULL,
                     scheduled_days REAL NOT NULL,
                     source TEXT NOT NULL DEFAULT 'flashcard'
                 );
                 CREATE TABLE user_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                     VALUES ('c1', '2024-01-01T00:00:00Z', 3, 0.0, 1.0);
                 INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days, source)
                     VALUES ('c2', '2024-06-15T12:30:00Z', 1, 0.0, 0.0, 'practice');
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        }

        let db = Database::open(&path_str, None).unwrap();
        let version: i64 = db
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4, "a v2 db converges through v3 to the current v4");

        // uuid: present, 36-char, unique per row
        let uuids = review_uuid_set(&db);
        assert_eq!(uuids.len(), 2);
        assert!(uuids.iter().all(|u| u.len() == 36), "uuids: {uuids:?}");

        // hlc parses; device_id set and equals this device's id
        let mut stmt = db
            .conn()
            .prepare("SELECT hlc, device_id FROM review_logs")
            .unwrap();
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        drop(stmt);
        let dev = device_id(&db);
        for (hlc, row_dev) in &rows {
            let parsed: Hlc = hlc.parse().expect("backfilled hlc must parse");
            assert_eq!(parsed.counter, 0);
            assert_eq!(parsed.node, hlc_node_of(&dev));
            assert_eq!(row_dev, &dev);
        }
        // Legacy timestamp → HLC ms: 2024-01-01T00:00:00Z = day 19723.
        let c1_hlc: String = db
            .conn()
            .query_row("SELECT hlc FROM review_logs WHERE card_id = 'c1'", [], |r| r.get(0))
            .unwrap();
        let c1: Hlc = c1_hlc.parse().unwrap();
        assert_eq!(c1.ms, 19723 * 86400 * 1000);

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 2. CONVERGENCE: real submit path on A; the same events applied in two
    // different shuffled orders on B and C yield identical memory_states and
    // identical review uuid sets.
    #[test]
    fn test_convergence_shuffle_orders_agree() {
        let db_a = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();
        for i in 0..40 {
            let card = format!("card-{:02}", i % 8);
            let rating = [3u8, 4, 1, 2, 3][i % 5];
            if i % 7 == 0 {
                engine
                    .submit_review_with_source(&db_a, &card, rating, "practice")
                    .unwrap();
            } else {
                engine.submit_review(&db_a, &card, rating).unwrap();
            }
        }
        let events = events_since(&db_a, None).unwrap();
        assert_eq!(events.len(), 40);

        let db_b = Database::open(":memory:", None).unwrap();
        let db_c = Database::open(":memory:", None).unwrap();
        let n_b = apply_review_events(&db_b, &shuffle(events.clone(), 0xA5A5B1)).unwrap();
        let n_c = apply_review_events(&db_c, &shuffle(events.clone(), 0x5EEDC0DE)).unwrap();
        assert_eq!((n_b, n_c), (40, 40));

        let snap_a = memory_snapshot(&db_a);
        assert_eq!(snap_a.len(), 8);
        assert_eq!(snap_a, memory_snapshot(&db_b), "A vs B must converge");
        assert_eq!(snap_a, memory_snapshot(&db_c), "A vs C must converge");
        assert_eq!(review_uuid_set(&db_a), review_uuid_set(&db_b));
        assert_eq!(review_uuid_set(&db_a), review_uuid_set(&db_c));
    }

    // 3. IDEMPOTENCY: applying the same batch twice inserts nothing the
    // second time and leaves memory_states untouched.
    #[test]
    fn test_apply_idempotent_replay() {
        let db_a = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();
        for i in 0..12 {
            engine
                .submit_review(&db_a, &format!("dup-{}", i % 4), (i % 4 + 1) as u8)
                .unwrap();
        }
        let events = events_since(&db_a, None).unwrap();

        let db_b = Database::open(":memory:", None).unwrap();
        assert_eq!(apply_review_events(&db_b, &events).unwrap(), 12);
        let before = memory_snapshot(&db_b);
        assert_eq!(
            apply_review_events(&db_b, &events).unwrap(),
            0,
            "second apply must be uuid-dedup'd"
        );
        assert_eq!(before, memory_snapshot(&db_b));
        // And the local fold itself is idempotent.
        assert_eq!(rebuild_memory_states(&db_b).unwrap(), 4);
        assert_eq!(before, memory_snapshot(&db_b));
    }

    fn user_setting(db: &Database, key: &str) -> String {
        db.conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .unwrap()
    }

    // 4. SETTINGS LWW: older hlc loses, newer wins, equal-ms tie broken by
    // counter; wins are visible through the user_settings read path.
    #[test]
    fn test_settings_lww() {
        let db = Database::open(":memory:", None).unwrap();
        setting_put(&db, "theme", "dark").unwrap();
        assert_eq!(user_setting(&db, "theme"), "dark");
        let stored = settings_since(&db, None).unwrap();
        let (_, _, hlc0) = &stored[0];
        let cur: Hlc = hlc0.parse().unwrap();

        // Older loses.
        assert!(!setting_merge(&db, "theme", "light", &bump_ms(hlc0, -1000)).unwrap());
        assert_eq!(user_setting(&db, "theme"), "dark");

        // Equal-ms, lower counter loses; higher counter wins (tie-break).
        let lower = Hlc { ms: cur.ms, counter: cur.counter, node: cur.node.clone() }.to_string();
        let higher = Hlc { ms: cur.ms, counter: cur.counter + 1, node: cur.node.clone() }.to_string();
        assert!(!setting_merge(&db, "theme", "light", &lower).unwrap());
        assert!(setting_merge(&db, "theme", "light", &higher).unwrap());
        assert_eq!(user_setting(&db, "theme"), "light");
        // Winning hlc lands in settings_sync too.
        let stored_hlc: String = db
            .conn()
            .query_row("SELECT hlc FROM settings_sync WHERE key = 'theme'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored_hlc, higher);

        // Newer wall time wins over the tie-break winner.
        assert!(
            setting_merge(&db, "theme", "solarized", &bump_ms(&higher, 60_000)).unwrap()
        );
        assert_eq!(user_setting(&db, "theme"), "solarized");
    }

    // 5. GATE LATCH round-trip: record_alphabet_gate stamps settings_sync; a
    // foreign latch with a newer hlc flips another device's gate to satisfied.
    #[test]
    fn test_gate_latch_roundtrip_through_sync() {
        let db_a = Database::open(":memory:", None).unwrap();
        crate::curriculum::Curriculum::record_alphabet_gate(
            &db_a,
            crate::curriculum::AlphabetGateMethod::CheckPassed,
        )
        .unwrap();
        let settings = settings_since(&db_a, None).unwrap();
        let gate_row = settings
            .iter()
            .find(|(k, _, _)| k == "alphabet_gate")
            .expect("alphabet_gate must appear in settings_since");
        assert_eq!(gate_row.1, "check_passed");
        assert!(settings.iter().any(|(k, _, _)| k == "alphabet_gate_at"));

        // Foreign device: latch NOT set locally, letters deck present and no
        // progress, so the gate is unsatisfied...
        let db_b = Database::open(":memory:", None).unwrap();
        let deck: Vec<crate::srs::Card> = (1..=43)
            .map(|i| crate::srs::Card {
                id: format!("letters-{i:03}"),
                front: String::new(),
                back: String::new(),
                chapter: 1,
                card_type: String::new(),
                pos: String::new(),
                tags: vec!["letters".to_string()],
            })
            .collect();
        assert!(!crate::curriculum::Curriculum::alphabet_gate_state(&db_b, &deck).satisfied);

        // ...until A's latch arrives (its hlc necessarily wins on an empty
        // key; use the exact synced row to exercise apply_settings too).
        let kv = vec![(
            "alphabet_gate".to_string(),
            gate_row.1.clone(),
            gate_row.2.clone(),
        )];
        assert_eq!(apply_settings(&db_b, &kv).unwrap(), 1);
        let state = crate::curriculum::Curriculum::alphabet_gate_state(&db_b, &deck);
        assert!(state.satisfied, "foreign latch must satisfy the gate");
        assert_eq!(
            state.method,
            Some(crate::curriculum::AlphabetGateMethod::CheckPassed)
        );
    }

    // 6. events_since cursor: paging by last-seen hlc reassembles the
    // complete ordered event set exactly once.
    #[test]
    fn test_events_since_cursor_pagination_reassembles_full_set() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();
        for i in 0..25 {
            engine
                .submit_review(&db, &format!("page-{}", i % 6), ((i % 4) + 1) as u8)
                .unwrap();
        }
        let full = events_since(&db, None).unwrap();
        assert_eq!(full.len(), 25);

        // Step the cursor one event at a time (page size 1, worst case).
        let mut cursor: Option<String> = None;
        let mut collected: Vec<ReviewEvent> = Vec::new();
        loop {
            let page = events_since(&db, cursor.as_deref()).unwrap();
            let Some(first) = page.first() else { break };
            collected.push(first.clone());
            cursor = Some(first.hlc.clone());
        }
        assert_eq!(collected.len(), 25);
        assert_eq!(
            collected.iter().map(|e| &e.uuid).collect::<Vec<_>>(),
            full.iter().map(|e| &e.uuid).collect::<Vec<_>>()
        );

        // A mid-stream cursor yields exactly the tail.
        let tail = events_since(&db, Some(&full[7].hlc)).unwrap();
        assert_eq!(
            tail.iter().map(|e| &e.uuid).collect::<Vec<_>>(),
            full[8..].iter().map(|e| &e.uuid).collect::<Vec<_>>()
        );
    }

    // 7. Practice-source rows flow through events/apply and mirror the
    // incremental submit path on both sides: today's incremental path runs
    // 'practice' rows through the same FSRS scheduling as flashcards, and
    // the fold reproduces exactly that (verified against A).
    #[test]
    fn test_practice_rows_sync_and_fold_like_incremental() {
        let db_a = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();
        // An interleaved flashcard/practice history on the same cards.
        engine.submit_review(&db_a, "mix-1", 3).unwrap();
        engine
            .submit_review_with_source(&db_a, "mix-1", 1, "practice")
            .unwrap();
        engine
            .submit_review_with_source(&db_a, "mix-1", 4, "practice")
            .unwrap();
        engine.submit_review(&db_a, "mix-2", 2).unwrap();
        engine
            .submit_review_with_source(&db_a, "mix-2", 3, "practice")
            .unwrap();

        let events = events_since(&db_a, None).unwrap();
        assert_eq!(events.len(), 5);
        assert!(events.iter().any(|e| e.source == "practice"));
        assert!(events.iter().any(|e| e.source == "flashcard"));

        let db_b = Database::open(":memory:", None).unwrap();
        assert_eq!(apply_review_events(&db_b, &events).unwrap(), 5);

        // Both sides agree, and the fold result equals what A's incremental
        // path produced (practice rows included in the card's history).
        assert_eq!(memory_snapshot(&db_a), memory_snapshot(&db_b));
        assert_eq!(
            review_uuid_set(&db_a),
            review_uuid_set(&db_b),
            "practice rows must travel with the event set"
        );
        let sources_b: Vec<String> = {
            let mut stmt = db_b
                .conn()
                .prepare("SELECT source FROM review_logs WHERE card_id = 'mix-1' ORDER BY hlc")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<std::result::Result<Vec<String>, _>>()
                .unwrap()
        };
        assert_eq!(sources_b, ["flashcard", "practice", "practice"]);
    }

    // 8. Foreign-injected garbage (a compromised peer writing plaintext events)
    //    must not poison the log or the fold: bad ratings, hostile card ids,
    //    unparseable HLCs are dropped; valid events in the same batch merge.
    #[test]
    fn test_apply_drops_malformed_events() {
        let db = Database::open(":memory:", None).unwrap();
        let hlc = Hlc { ms: 1_760_000_000_000, counter: 0, node: "abcd1234".into() }.to_string();
        let base = ReviewEvent {
            uuid: uuid::Uuid::new_v4().to_string(),
            card_id: "athenaze-i-0001".into(),
            timestamp: "2025-10-09T12:00:00Z".into(),
            rating: 3,
            elapsed_days: 0.0,
            scheduled_days: 1.0,
            source: "flashcard".into(),
            device_id: "deadbeefcafe".into(),
            hlc: hlc.clone(),
        };
        let bad_rating = ReviewEvent { rating: 9, uuid: uuid::Uuid::new_v4().to_string(), ..base.clone() };
        let bad_zero = ReviewEvent { rating: 0, uuid: uuid::Uuid::new_v4().to_string(), ..base.clone() };
        let bad_card = ReviewEvent { card_id: "x'; DROP TABLE memory_states;--".into(), uuid: uuid::Uuid::new_v4().to_string(), ..base.clone() };
        let bad_hlc = ReviewEvent { hlc: "not-an-hlc".into(), uuid: uuid::Uuid::new_v4().to_string(), ..base.clone() };
        let good = ReviewEvent { uuid: uuid::Uuid::new_v4().to_string(), ..base };

        let inserted = apply_review_events(&db, &[bad_rating, bad_zero, bad_card, bad_hlc, good.clone()]).unwrap();
        assert_eq!(inserted, 1);

        let uuids: Vec<String> = {
            let mut s = db.conn().prepare("SELECT uuid FROM review_logs").unwrap();
            s.query_map([], |r| r.get(0)).unwrap().collect::<std::result::Result<_, _>>().unwrap()
        };
        assert_eq!(uuids, vec![good.uuid]);

        // The surviving event still folds into memory_states.
        let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM memory_states", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    // 9. SECRETS VAULT roundtrip: put/get/has/delete against the `secrets`
    //    table (schema v4), including overwrite semantics.
    #[test]
    fn test_secrets_roundtrip() {
        let db = Database::open(":memory:", None).unwrap();
        assert!(!secret_has(&db, "nostr_nsec"));
        assert_eq!(secret_get(&db, "nostr_nsec").unwrap(), None);

        secret_put(&db, "nostr_nsec", "hex-aaaa").unwrap();
        assert!(secret_has(&db, "nostr_nsec"));
        assert_eq!(secret_get(&db, "nostr_nsec").unwrap().as_deref(), Some("hex-aaaa"));

        // Upsert semantics.
        secret_put(&db, "nostr_nsec", "hex-bbbb").unwrap();
        assert_eq!(secret_get(&db, "nostr_nsec").unwrap().as_deref(), Some("hex-bbbb"));

        secret_delete(&db, "nostr_nsec").unwrap();
        assert!(!secret_has(&db, "nostr_nsec"));
        // Delete of an absent key is a no-op, not an error.
        secret_delete(&db, "nostr_nsec").unwrap();
    }

    // 10. BLOCKLIST: setting_put refuses keys matching *nsec* (any case)
    //     and values matching the nsec1 bech32 pattern; plausible-but-
    //     innocent keys/values still pass.
    #[test]
    fn test_setting_put_rejects_nsec_keys_and_values() {
        let db = Database::open(":memory:", None).unwrap();
        let nsec_val = format!("nsec1{}", "q".repeat(58));

        for bad_key in ["nostr_nsec", "NSEC_backup", "other_nsec2"] {
            let err = setting_put(&db, bad_key, "harmless").unwrap_err();
            assert!(err.to_string().contains("blocklist"), "{bad_key}: {err}");
        }
        let err = setting_put(&db, "innocent_key", &nsec_val).unwrap_err();
        assert!(err.to_string().contains("nsec1"), "{err}");
        // Embedded anywhere in the value, not just at the start.
        let err = setting_put(&db, "blob", &format!("prefix {nsec_val} suffix")).unwrap_err();
        assert!(err.to_string().contains("nsec1"), "{err}");

        // Nothing blocked was written to either settings table (fresh DBs
        // legitimately hold bootstrap keys like device_id, so count only
        // the keys under test).
        for table in ["user_settings", "settings_sync"] {
            let n: i64 = db
                .conn()
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table}
                         WHERE key IN ('nostr_nsec', 'NSEC_backup', 'other_nsec2',
                                       'innocent_key', 'blob')"
                    ),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{table} must hold none of the refused rows");
        }

        // Innocent writes still work (incl. short nsec-looking strings).
        setting_put(&db, "theme", "dark").unwrap();
        setting_put(&db, "note", "nsec1tooshort").unwrap();
        assert_eq!(user_setting(&db, "theme"), "dark");
    }

    // 11. BLOCKLIST via the MERGE path: a remote batch (or imported backup)
    //     cannot re-plant a secret — blocked rows are dropped pre-HLC-check,
    //     innocent rows in the same batch still merge.
    #[test]
    fn test_setting_merge_cannot_bypass_blocklist() {
        let db = Database::open(":memory:", None).unwrap();
        let hlc = Hlc { ms: 1_800_000_000_000, counter: 7, node: "cafebabe".into() }.to_string();
        let nsec_val = format!("nsec1{}", "q".repeat(58));

        let kv = vec![
            ("nostr_nsec".to_string(), "deadbeef".to_string(), hlc.clone()),
            ("blob".to_string(), format!("payload {nsec_val}"), hlc.clone()),
            ("theme".to_string(), "solarized".to_string(), hlc.clone()),
        ];
        // First two rows fail the blocklist (bad key; bad value), the
        // innocent row in the same batch still applies.
        assert_eq!(apply_settings(&db, &kv).unwrap(), 1);
        assert_eq!(user_setting(&db, "theme"), "solarized");
        for table in ["user_settings", "settings_sync"] {
            let n: i64 = db
                .conn()
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {table} WHERE key IN ('nostr_nsec', 'blob')"
                    ),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{table} must never receive blocked rows");
        }
    }

    // 12. settings_since REGRESSION: the sync view must never surface
    //     secrets rows (they live in a different table entirely — this test
    //     pins that even a secret named like a synced key stays invisible).
    #[test]
    fn test_settings_since_never_returns_secrets() {
        let db = Database::open(":memory:", None).unwrap();
        secret_put(&db, "nostr_nsec", "hex-secret").unwrap();
        secret_put(&db, "theme", "this-is-a-secret-not-a-setting").unwrap();
        setting_put(&db, "theme", "light").unwrap();

        let rows = settings_since(&db, None).unwrap();
        assert_eq!(rows.len(), 1, "only the real setting may be visible: {rows:?}");
        assert_eq!(rows[0].0, "theme");
        assert_eq!(rows[0].1, "light");
        assert!(!rows.iter().any(|(_, v, _)| v.contains("secret")));
    }
}
