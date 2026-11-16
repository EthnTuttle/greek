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

/// Observe a REMOTE HLC: advance the persisted `hlc_last_ms` /
/// `hlc_last_counter` to the maximum of the stored values and the incoming
/// `(ms, counter)` — monotonic, never rewinds — so the next [`hlc_now`]
/// stamps strictly after anything this device has SEEN, not just anything
/// it WROTE. Standard HLC receive-side rule: without it, a local review
/// written after applying an ahead-of-clock remote review sorts BEFORE the
/// event it followed, and replicas fold identical event sets in different
/// `(hlc, uuid)` orders into different memory states (S4: probe showed
/// stability 0.949919 vs 7.861664 for the same event set).
///
/// Counter semantics mirror [`hlc_now`]: a strictly newer `ms` adopts the
/// remote counter verbatim; an equal `ms` keeps the max counter. The node
/// tag is NOT adopted — identity stays local. Direct upserts like
/// [`hlc_now`] (never `setting_put`) so the device-local keys keep out of
/// `settings_sync`, and `hlc_now`'s read-modify-write picks the observation
/// up on the next stamp (same ms → counter+1 dominates the observed remote
/// event). Safe inside an enclosing `with_tx`: writes ride that envelope.
/// Returns `true` when the stored values moved.
pub fn hlc_observe(db: &Database, remote_hlc: &str) -> Result<bool> {
    let remote: Hlc = remote_hlc.parse()?;

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
    let remote_counter = remote.counter as i64;

    let (ms, counter) = if remote.ms > last_ms {
        (remote.ms, remote_counter)
    } else if remote.ms == last_ms && remote_counter > last_counter {
        (last_ms, remote_counter)
    } else {
        // Older (or equal): do not rewind.
        return Ok(false);
    };

    let write = |key: &str, value: i64| {
        let _ = db.conn().execute(
            "INSERT INTO user_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value.to_string()],
        );
    };
    write("hlc_last_ms", ms);
    write("hlc_last_counter", counter);
    Ok(true)
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

/// One `known_words` row on the wire (v2, SYNC-WIRE.md §2.1): the lemma or
/// surface form the user marked, its known flag, and the canonical RFC3339
/// write time that arbitrates LWW across devices (`updated_at` is written by
/// `srs::now_iso8601`, i.e. canonical UTC, so string compare == chronology).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KnownWordEntry {
    pub word: String,
    pub known: bool,
    pub updated_at: String,
}

/// One `lookup_cards` content row on the wire (v2): user-created card
/// content (front = the Greek word, back = the definition). Its presence
/// alone is the card's existence — unreviewed lookup cards carry NO
/// `memory_states` row (schema v5), so nothing else must ride the wire.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LookupCardEntry {
    pub card_id: String,
    pub front: String,
    pub back: String,
}

/// One manual chapter unlock on the wire (v2): `(textbook, chapter)` with
/// `unlocked = 1` in `curriculum_progress`. Locking never travels (the app
/// has no relock path), so the unlock set is a grow-only CRDT by PK dedup.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChapterUnlock {
    pub textbook: String,
    pub chapter: i32,
}

/// Highest sync wire version this build accepts (SYNC-WIRE.md §2.1).
/// v1 = legacy batches (`reviews` + `settings` only); v2 adds the
/// `known_words` / `lookup_cards` / `chapter_unlocks` collections.
pub const SYNC_WIRE_VERSION_MAX: u32 = 2;

/// A sync batch — the sync wire format (SYNC-WIRE.md §2.1), the canonical
/// type both transports (nostr spool, iroh-docs) and the backup
/// export/import path serialize.
///
/// VERSIONING: `v` is 1 for legacy payloads (only `reviews` + `settings`
/// exist) and 2 for full payloads. All three v2 collections are
/// `#[serde(default)]`, so a v1 document deserializes with them EMPTY — a
/// v1 import therefore applies reviews+settings only, exactly the legacy
/// behavior. A missing `v` is read as 1 (`default_wire_version`). Anything
/// above [`SYNC_WIRE_VERSION_MAX`] is unknown future data and is REJECTED
/// whole by [`validate_batch`] at every ingress point (backup import and
/// the nostr reassembly; iroh carries events entry-wise, not batches).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SyncBatch {
    /// Wire format version: 1 = legacy (reviews + settings), 2 = full.
    #[serde(default = "default_wire_version")]
    pub v: u32,
    pub device_id: String,
    pub from_hlc: String,
    pub to_hlc: String,
    pub reviews: Vec<ReviewEvent>,
    /// (key, value, hlc) settings triples.
    pub settings: Vec<(String, String, String)>,
    /// v2: all `known_words` rows (full set every batch — small table, no
    /// HLC column exists on it, so incremental cursors cannot apply).
    #[serde(default)]
    pub known_words: Vec<KnownWordEntry>,
    /// v2: all `lookup_cards` content rows (full set; ID-dedup at ingress).
    #[serde(default)]
    pub lookup_cards: Vec<LookupCardEntry>,
    /// v2: all unlocked chapters (full set; grow-only, PK-dedup at ingress).
    #[serde(default)]
    pub chapter_unlocks: Vec<ChapterUnlock>,
}

/// Missing `v` == v1 (pre-versioning payloads).
fn default_wire_version() -> u32 {
    1
}

/// Whole-batch version gate shared by every batch ingress (backup-file
/// import, nostr reassembly). Versions 1 and 2 are accepted; anything else
/// (0, or a future version this build cannot interpret) is REJECTED as a
/// unit — never partially applied.
pub fn validate_batch(batch: &SyncBatch) -> Result<()> {
    if batch.v == 0 || batch.v > SYNC_WIRE_VERSION_MAX {
        anyhow::bail!(
            "unsupported sync wire version {} (this app speaks v1-v{SYNC_WIRE_VERSION_MAX})",
            batch.v
        );
    }
    Ok(())
}

/// Card-id upper bound, in CHARS (Unicode scalar values), applied before
/// and after NFC normalization. Deck ids today are short ASCII slugs and
/// lookup ids are `lookup-<lemma>`; 256 chars is far beyond any real id
/// and still bounds relay/import payloads.
pub const CARD_ID_MAX_LEN: usize = 256;

/// Card-id ingress gate (SYNC-WIRE.md §2.1, S1): returns the NFC-canonical
/// form of a plausible id, `None` to DROP it.
///
/// Rules — deliberately permissive about Unicode (genuine app-created ids
/// contain Greek, e.g. `lookup-λόγος`), strict about everything that can
/// break storage or tooling:
/// - non-empty, at most [`CARD_ID_MAX_LEN`] chars (both before and after
///   normalization);
/// - no ASCII control bytes (`is_ascii_control`: NUL…US and DEL) — those
///   break logs, dumps and line-oriented tooling;
/// - no `'`, `"` or `\` — identifier-breaking bytes for SQL/JSON/shell
///   contexts (SQLite itself is safe via bound params; the gate exists so
///   nothing downstream ever has to re-learn that).
///
/// Everything else — Greek letters and accents, CJK, combining marks — is
/// accepted and NFC-normalized here, so a sender whose string is NFD-folded
/// and one in NFC agree on ONE wire id (the ID gate is shared by backup
/// import and both transport pulls: they all call [`apply_review_events`]).
pub fn normalize_card_id(id: &str) -> Option<String> {
    use unicode_normalization::UnicodeNormalization;
    if id.is_empty() || id.chars().count() > CARD_ID_MAX_LEN {
        return None;
    }
    if id
        .chars()
        .any(|c| c.is_ascii_control() || matches!(c, '\'' | '"' | '\\'))
    {
        return None;
    }
    let nfc: String = id.nfc().collect();
    if nfc.is_empty() || nfc.chars().count() > CARD_ID_MAX_LEN {
        return None;
    }
    Some(nfc)
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

// --- Push cursors, destination-scoped (S7) ---
//
// Push cursors are device-local high-water marks in `user_settings`
// (written directly, never via `settings_sync`/LWW — cross-device LWW on a
// drifting clock could rewind a cursor past unsent local events forever).
// Each cursor key is scoped to its *destination* by a fingerprint suffix:
//
//     nostr_last_push_hlc@<fingerprint>   — fingerprint over the normalized
//                                           relay set + identity pubkey hex
//     iroh_last_push_hlc@<fingerprint>    — fingerprint of the notebook's
//                                           namespace (doc) id
//
// Switching relays/identity/notebook therefore yields a destination with NO
// cursor row, so the next push to it is a FULL push (S7-b); the previous
// destination's row is kept untouched, so switching back resumes its
// incremental cursor (no redundant full resend).
//
// Pre-S7 databases carry the UNSCOPED rows (`nostr_last_push_hlc`,
// `iroh_last_push_hlc`). To avoid one full re-publish on upgrade, the first
// [`push_cursor_get`] after the upgrade migrates a legacy row into the
// then-current destination's scoped key — once, guarded by the
// `{prefix}@migrated` marker row, so a LATER switch to a different
// destination can never inherit it. The legacy row itself is kept
// (harmless, and a raw fallback for downgrades).
//
// Known residual gap: fingerprinting only covers the fields above — a
// different relay operator/INSTALL serving the same relay URL set under the
// same identity is treated as the same destination. Acceptable: URLs define
// the destination contract.

/// Push-cursor prefixes in `user_settings`; scoped keys append
/// `@{fingerprint}` ([`push_cursor_key`]).
pub const NOSTR_PUSH_CURSOR_PREFIX: &str = "nostr_last_push_hlc";
pub const IROH_PUSH_CURSOR_PREFIX: &str = "iroh_last_push_hlc";
/// Marker recording that the legacy unscoped cursor was already migrated.
const PUSH_CURSOR_MIGRATED_SUFFIX: &str = "@migrated";

/// Fingerprint a sync destination: 64-bit FNV-1a over `parts` (each part
/// length-prefixed so concatenation ambiguities can't alias), rendered as 16
/// hex chars. Non-cryptographic by design — it only partitions cursor rows.
pub fn destination_fingerprint(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for part in parts {
        mix(&(part.len() as u64).to_le_bytes());
        mix(part.as_bytes());
    }
    format!("{h:016x}")
}

/// The `user_settings` cursor key for `prefix` at destination `fingerprint`.
pub fn push_cursor_key(prefix: &str, fingerprint: &str) -> String {
    format!("{prefix}@{fingerprint}")
}

fn user_setting_read(db: &Database, key: &str) -> Option<String> {
    db.conn()
        .query_row(
            "SELECT value FROM user_settings WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        )
        .ok()
}

fn user_setting_upsert(db: &Database, key: &str, value: &str) -> Result<()> {
    db.conn().execute(
        "INSERT INTO user_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Read the push cursor for the CURRENT destination (`fingerprint`).
///
/// Read-through legacy migration: when the scoped row is absent and the
/// one-time `{prefix}@migrated` marker is also absent, a legacy UNSCOPED
/// `prefix` row (pre-S7) is copied into the scoped key — preserving
/// incremental continuity across the upgrade — and the marker is written
/// either way so no other destination can ever inherit it afterwards.
pub fn push_cursor_get(db: &Database, prefix: &str, fingerprint: &str) -> Option<String> {
    let scoped = push_cursor_key(prefix, fingerprint);
    if let Some(v) = user_setting_read(db, &scoped) {
        return Some(v);
    }
    let marker = format!("{prefix}{PUSH_CURSOR_MIGRATED_SUFFIX}");
    if user_setting_read(db, &marker).is_some() {
        return None;
    }
    let legacy = user_setting_read(db, prefix);
    if let Some(v) = &legacy {
        let _ = user_setting_upsert(db, &scoped, v);
    }
    let _ = user_setting_upsert(db, &marker, "1");
    legacy
}

/// Store the destination-scoped push cursor (device-local upsert).
pub fn push_cursor_put(db: &Database, prefix: &str, fingerprint: &str, hlc: &str) -> Result<()> {
    user_setting_upsert(db, &push_cursor_key(prefix, fingerprint), hlc)
}

/// S7 import rewind: a backup import can merge review rows whose HLCs
/// PREDATE a destination's push cursor (the destination never saw them —
/// they arrived via file, not the wire). For every push cursor (scoped or
/// legacy) sitting at-or-above `min_hlc`, rewind to just-below it so the
/// next push to that destination re-ships the merged history. Receivers
/// uuid-dedup, so resending already-delivered rows is safe.
///
/// Pull cursors and the migration markers are never touched. Returns how
/// many cursor rows were rewound.
pub fn rewind_push_cursors_below(db: &Database, min_hlc: &str) -> Result<u32> {
    // HLCs are fixed-width lexicographic (`{ms:013}-{counter:04}-{node}`),
    // so plain string comparison is logical ordering. "Just below"
    // `min_hlc`: one millisecond earlier with minimal counter/node — still
    // strictly below any row at `min_hlc`, so the merged minimum row itself
    // is re-selected (selection is `hlc > cursor`). An unparseable cursor
    // comparator falls back to "" (full push for that destination — the
    // safe direction).
    let below: String = match min_hlc.parse::<Hlc>() {
        Ok(h) => Hlc {
            ms: (h.ms - 1).max(0),
            counter: 0,
            node: "00000000".to_string(),
        }
        .to_string(),
        Err(_) => String::new(),
    };
    let mut stmt = db
        .conn()
        .prepare("SELECT key, value FROM user_settings")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let is_cursor_key = |k: &str| {
        (k.starts_with(NOSTR_PUSH_CURSOR_PREFIX) || k.starts_with(IROH_PUSH_CURSOR_PREFIX))
            && !k.ends_with(PUSH_CURSOR_MIGRATED_SUFFIX)
    };
    let mut n = 0u32;
    for (key, value) in rows {
        if is_cursor_key(&key) && !value.is_empty() && value.as_str() >= min_hlc {
            user_setting_upsert(db, &key, &below)?;
            n += 1;
        }
    }
    Ok(n)
}

// --- S6 pull admission ledger (`sync_applied_batches`, schema v6) ---
//
// Nostr pull admission is per-batch UUID (the chunk "b" tag), NOT the
// single global `nostr_last_pull_hlc` cursor (that cursor remains only as
// an observability high-water mark). A batch applies iff its UUID has no
// row here — at ANY HLC position — so a batch that was incomplete when the
// cursor advanced past it (or pushed from a lagging clock) still merges
// when its chunks later complete. Replay safety below the ledger comes
// from uuid-dedup in `apply_review_events` / PK-dedup content applies.

/// Record a wire batch as APPLIED. Call in the SAME transaction as the
/// batch's application (`db.with_tx` around the applies + this mark), so a
/// crash mid-apply leaves the UUID unmarked and the batch retry-able.
/// Idempotent (INSERT OR IGNORE): re-marking an applied batch is a no-op.
pub fn mark_batch_applied(db: &Database, batch_uuid: &str) -> Result<()> {
    db.conn().execute(
        "INSERT OR IGNORE INTO sync_applied_batches (batch_uuid, applied_at)
         VALUES (?1, ?2)",
        rusqlite::params![batch_uuid, crate::srs::now_iso8601()],
    )?;
    Ok(())
}

/// Has this wire batch UUID already been applied on this device?
pub fn batch_is_applied(db: &Database, batch_uuid: &str) -> bool {
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM sync_applied_batches WHERE batch_uuid = ?1",
            [batch_uuid],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

// --- v2 content collections (SYNC-WIRE.md §2.1) ---
//
// `known_words`, `lookup_cards` and `curriculum_progress` carry no HLC
// columns, so batches always ship the FULL row sets (the tables are small:
// one row per known word / lookup-created card / unlocked chapter).
// Convergence does not need cursors: known-words merge is LWW on
// `updated_at`, lookup cards and chapter unlocks are dedup-inserts by PK.

/// All `known_words` rows (the v2 export projection), ordered by word.
pub fn known_words_all(db: &Database) -> Result<Vec<KnownWordEntry>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT word, known, updated_at FROM known_words ORDER BY word")?;
    let rows = stmt.query_map([], |row| {
        Ok(KnownWordEntry {
            word: row.get(0)?,
            known: row.get::<_, i64>(1)? != 0,
            updated_at: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// All `lookup_cards` content rows, ordered by card_id.
pub fn lookup_cards_all(db: &Database) -> Result<Vec<LookupCardEntry>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT card_id, front, back FROM lookup_cards ORDER BY card_id")?;
    let rows = stmt.query_map([], |row| {
        Ok(LookupCardEntry {
            card_id: row.get(0)?,
            front: row.get(1)?,
            back: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// All manually unlocked chapters (`unlocked != 0`), ordered for stable
/// wire output.
pub fn chapter_unlocks_all(db: &Database) -> Result<Vec<ChapterUnlock>> {
    let mut stmt = db.conn().prepare(
        "SELECT textbook, chapter FROM curriculum_progress
         WHERE unlocked != 0 ORDER BY textbook, chapter",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ChapterUnlock {
            textbook: row.get(0)?,
            chapter: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Size cap for lookup-card `front`/`back` content at ingress (chars).
/// Realistic content is a word plus a short definition; this bounds hostile
/// payloads without touching legitimate ones.
const LOOKUP_CONTENT_MAX_LEN: usize = 4096;

/// Merge incoming known-words rows: per-word LWW on the canonical
/// `updated_at` string. Skip-and-count ingress: rows with an
/// implausible word (see [`normalize_card_id`] — same rules: words are
/// lemma identifiers) or a non-canonical `updated_at` are DROPPED.
/// Returns rows applied.
pub fn apply_known_words(db: &Database, entries: &[KnownWordEntry]) -> Result<u32> {
    db.with_tx(|conn| {
        let mut applied = 0u32;
        for e in entries {
            let Some(word) = normalize_card_id(&e.word) else {
                continue;
            };
            if !is_canonical_timestamp(&e.updated_at) {
                continue;
            }
            let existing: Option<String> = conn
                .query_row(
                    "SELECT updated_at FROM known_words WHERE word = ?1",
                    [&word],
                    |row| row.get(0),
                )
                .ok();
            // LWW: a strictly-newer (or absent) write wins. Equal timestamps
            // keep the incumbent — convergent because the content at an
            // equal (word, updated_at) is identical on every writer.
            if existing.as_deref().is_some_and(|cur| cur >= e.updated_at.as_str()) {
                continue;
            }
            conn.execute(
                "INSERT INTO known_words (word, known, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(word) DO UPDATE SET known = excluded.known,
                                                 updated_at = excluded.updated_at",
                rusqlite::params![word, e.known as i64, e.updated_at],
            )?;
            applied += 1;
        }
        Ok(applied)
    })
}

/// Merge incoming lookup-card content rows: dedup by card_id (first writer
/// wins — the same lookup regenerates the same content, so a conflict is
/// almost always an exact duplicate). Skip-and-count: rows whose card_id
/// fails [`normalize_card_id`] (NFC-normalized here, matching the review
/// gate) or whose content is oversized are DROPPED. Returns rows applied.
pub fn apply_lookup_cards(db: &Database, entries: &[LookupCardEntry]) -> Result<u32> {
    db.with_tx(|conn| {
        let mut applied = 0u32;
        for e in entries {
            let Some(card_id) = normalize_card_id(&e.card_id) else {
                continue;
            };
            if e.front.chars().count() > LOOKUP_CONTENT_MAX_LEN
                || e.back.chars().count() > LOOKUP_CONTENT_MAX_LEN
            {
                continue;
            }
            applied += conn.execute(
                "INSERT INTO lookup_cards (card_id, front, back)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(card_id) DO NOTHING",
                rusqlite::params![card_id, e.front, e.back],
            )? as u32;
        }
        Ok(applied)
    })
}

/// Merge incoming manual chapter unlocks: dedup by `(textbook, chapter)`.
/// Unlocking is a grow-only set (the app has no relock path), so a plain
/// PK-dedup insert IS the convergent merge. Skip-and-count: implausible
/// rows (empty/overlong textbook slug, negative or absurdly large chapter)
/// are DROPPED. Returns rows applied.
pub fn apply_chapter_unlocks(db: &Database, entries: &[ChapterUnlock]) -> Result<u32> {
    db.with_tx(|conn| {
        let mut applied = 0u32;
        for e in entries {
            let textbook_ok = !e.textbook.is_empty()
                && e.textbook.len() <= 64
                && e.textbook
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
            if !textbook_ok || !(0..=10000).contains(&e.chapter) {
                continue;
            }
            applied += conn.execute(
                "INSERT INTO curriculum_progress (textbook, chapter, unlocked)
                 VALUES (?1, ?2, 1)
                 ON CONFLICT(textbook, chapter) DO NOTHING",
                rusqlite::params![e.textbook, e.chapter],
            )? as u32;
        }
        Ok(applied)
    })
}

/// Strict canonical RFC3339 UTC check: exact `YYYY-MM-DDTHH:MM:SSZ` shape
/// (20 bytes, `T` at index 10, `Z` terminator, fixed `-`/`:` separators),
/// the date part validated by [`crate::srs::parse_iso_ymd`] (month 1..=12,
/// day 1..=31, year != 0), HH 0..=23, MM/SS 0..=59. No offsets, no
/// fractional seconds, no leap seconds — this is the only shape the fold
/// consumers ([`crate::srs::days_between`] / [`crate::srs::add_days_to_now`])
/// treat as deterministic input; anything else is rejected at ingress.
fn is_canonical_timestamp(ts: &str) -> bool {
    let b = ts.as_bytes();
    if b.len() != 20 || b[10] != b'T' || b[19] != b'Z' || b[13] != b':' || b[16] != b':' {
        return false;
    }
    if crate::srs::parse_iso_ymd(ts).is_none() {
        return false;
    }
    let two = |i: usize| -> Option<u32> {
        let (hi, lo) = (b[i], b[i + 1]);
        if hi.is_ascii_digit() && lo.is_ascii_digit() {
            Some(((hi - b'0') * 10 + (lo - b'0')) as u32)
        } else {
            None
        }
    };
    match (two(11), two(14), two(17)) {
        (Some(h), Some(m), Some(s)) => h <= 23 && m <= 59 && s <= 59,
        _ => false,
    }
}

/// Merge remote review events: uuid-dedup INSERT (FSRS state is NOT
/// incrementally updated per event), then one deterministic fold via
/// [`rebuild_memory_states`]. Returns the number of newly APPLIED
/// (non-duplicate) rows.
///
/// ATOMICITY (S5): validate FIRST, mutate SECOND — every event is screened
/// into `plausible` / dropped (skip-and-count) before any write happens,
/// then the inserts AND the fold run together in ONE `BEGIN IMMEDIATE`
/// transaction ([`Database::with_tx`]). Atomicity is all-or-nothing by
/// design: a crash or a fold error rolls the batch's inserts back WITH the
/// aborted fold, so the database can never hold review logs without their
/// folded memory states, and a retry re-attempts the full batch instead of
/// dup-skipping the poisoned rows out of repair. On `Err` NOTHING from the
/// batch was applied; on `Ok(n)`, `n` rows landed AND folded. (When an
/// enclosing caller already opened a transaction — e.g. the backup-import
/// wrapper — this joins that envelope instead of opening a nested one.)
///
/// Events can arrive over untrusted transports (a relay stores ciphertext,
/// but a compromised peer could inject plaintext): DROP anything that is
/// not a plausible review — rating must be 1..=4 (FSRS grades), card_id
/// must pass the Unicode-aware ingress gate ([`normalize_card_id`], which
/// also NFC-canonicalizes the id so NFC and NFD spellings of e.g.
/// `lookup-λόγος` converge to ONE card), the timestamp must be canonical
/// RFC3339 UTC (see [`is_canonical_timestamp`], so the fold's date math
/// never sees malformed input), the HLC must parse — instead of letting
/// garbage poison the fold.
pub fn apply_review_events(db: &Database, events: &[ReviewEvent]) -> Result<u32> {
    // Screen the whole batch BEFORE any mutation (skip-and-count: garbage
    // is dropped silently, never inserted, never folded). Surviving events
    // are cloned with their card_id NFC-normalized.
    let plausible: Vec<ReviewEvent> = events
        .iter()
        .filter_map(|e| {
            if !(1..=4).contains(&e.rating)
                || !is_canonical_timestamp(&e.timestamp)
                || e.hlc.parse::<Hlc>().is_err()
            {
                return None;
            }
            let card_id = normalize_card_id(&e.card_id)?;
            Some(ReviewEvent {
                card_id,
                ..e.clone()
            })
        })
        .collect();
    // S4: the batch max HLC this device is about to SEE (lexicographic
    // order == logical order). Observed below inside the same tx as the
    // inserts/fold — even when every row is a duplicate, so a replayed
    // ahead-of-clock batch still repairs the local clock (the dup-skip
    // path must not skip the clock repair that lets a later local event
    // sort after these).
    let max_event_hlc = plausible.iter().map(|e| e.hlc.as_str()).max();

    db.with_tx(|conn| {
        if let Some(h) = max_event_hlc {
            hlc_observe(db, h)?;
        }
        let mut inserted = 0u32;
        for e in &plausible {
            let n = conn.execute(
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
            // `db.conn()` IS `conn`; the fold's reads and writes join this
            // same transaction, so an error here rolls the inserts back too.
            rebuild_memory_states(db)?;
        }
        Ok(inserted)
    })
}

/// LWW-merge settings rows `(key, value, hlc)`; returns rows applied.
///
/// Also OBSERVES the batch max HLC (S4, [`hlc_observe`]): merged or not,
/// a well-formed incoming HLC is a clock this device has seen, so the next
/// local write must sort after it. Rows with unparseable HLCs keep their
/// existing behavior (`setting_merge` errors only once actually compared);
/// they simply cannot move the clock.
pub fn apply_settings(db: &Database, kv: &[(String, String, String)]) -> Result<u32> {
    let mut applied = 0u32;
    let mut max_hlc: Option<&str> = None;
    for (key, value, hlc) in kv {
        if hlc.parse::<Hlc>().is_ok() && max_hlc < Some(hlc.as_str()) {
            max_hlc = Some(hlc.as_str());
        }
        if setting_merge(db, key, value, hlc)? {
            applied += 1;
        }
    }
    if let Some(h) = max_hlc {
        hlc_observe(db, h)?;
    }
    Ok(applied)
}

/// THE FOLD: for each card with review history, replay all events in total
/// order `(hlc, uuid)` and recompute FSRS state EXACTLY as the incremental
/// submit path (`SrsEngine::submit_review_with_source`) would have:
/// whole-day elapsed gaps against the previous event's timestamp, default
/// desired retention 0.9, rating outside 1-4 treated as Good, due date =
/// event timestamp + interval, overridden by the learning-step ladder
/// (`srs::step_due`) for Again ratings — identical to the incremental submit
/// path. `source` participates in replay: a graduated-card practice miss
/// preserves its prior memory exactly as in the incremental path. All other
/// review/source combinations select their normal FSRS next state. Writes
/// the resulting memory_states rows; rows for cards with NO review history
/// are untouched. Idempotent.
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
        let events: Vec<(String, i64, String)> = {
            let mut stmt = db.conn().prepare(
                "SELECT timestamp, rating, source FROM review_logs
                 WHERE card_id = ?1
                 ORDER BY hlc, uuid, id",
            )?;
            let v = stmt
                .query_map([card_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
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

        for (ts, rating, source) in &events {
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
            if !crate::srs::preserve_memory_for_practice_miss(
                source,
                *rating as u8,
                reps as i32,
                lapses as i32,
            ) {
                stability = chosen.memory.stability;
                difficulty = chosen.memory.difficulty;
            }
            // Learning-step mirror: use pre-review rep/lapse counts exactly
            // like the incremental submit path, so a card whose final review
            // was an in-learning Again converges to the same short-step due.
            due_date = crate::srs::step_due(reps as i32, lapses as i32, *rating as u8, chosen.interval, ts);
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

/// Allowlist of settings keys that are legitimately portable across devices.
/// Anything NOT on this list is device-local state (identity, HLC cursors,
/// sync push/pull watermarks) or local compute cache, and must never be
/// accepted from an incoming batch — a foreign write to e.g. `device_id`
/// would poison the local identity, and to `*_hlc` cursors would corrupt
/// the sync watermarks.
///
/// Evidence for each entry (the write path of every synced key):
/// - `theme`, `new_per_day`: portable UI/SRS preferences (the wire-format
///   example keys, SYNC-WIRE.md §2.1 / sync-nostr codec).
/// - `nostr_relays`: relay list the user configures once and wants on every
///   paired device (`uniffi-bindings` writes it via `setting_put`).
/// - `alphabet_gate`, `alphabet_gate_at`: the cross-device gate latch
///   (`curriculum::Curriculum::record_alphabet_gate`).
/// - `share.enabled`, `share.relays`, `share.template_locale` and the
///   `share.milestone.*` / `share.template.*` / `share.posted.*` families:
///   `share-nostr` writes each via `setting_put` precisely so they LWW-sync.
///
/// Keys deliberately EXCLUDED: `device_id`, `hlc_last_ms`,
/// `hlc_last_counter`, `nostr_last_push_hlc`, `nostr_last_pull_hlc`,
/// `iroh_last_push_hlc` (identity/sync cursors, written straight to
/// `user_settings` bypassing `settings_sync`), `share.streak.cache`
/// (local ephemeral cache; `detect.rs` asserts it never reaches
/// `settings_sync`), and anything secret-shaped (also caught by
/// [`secret_blocklist_reason`] as defense-in-depth).
fn is_portable_setting(key: &str) -> bool {
    const EXACT: &[&str] = &[
        "theme",
        "new_per_day",
        "nostr_relays",
        "alphabet_gate",
        "alphabet_gate_at",
        "share.enabled",
        "share.relays",
        "share.template_locale",
    ];
    const PREFIXES: &[&str] = &["share.milestone.", "share.template.", "share.posted."];
    EXACT.contains(&key) || PREFIXES.iter().any(|p| key.starts_with(p))
}

/// LWW merge for an incoming `(key, value, hlc)`: a strictly-greater incoming
/// HLC wins; on a win BOTH tables are written. Returns whether it was applied.
///
/// Ingress is allowlisted (see [`is_portable_setting`]): an incoming row for
/// any other key — `device_id`, HLC/push cursors, local caches — is DROPPED
/// before any HLC comparison, so no remote batch or imported backup can
/// overwrite local-only device state. Rows shaped like a nostr secret (see
/// [`secret_blocklist_reason`]) are likewise DROPPED, so the blocklist
/// cannot be bypassed either.
pub fn setting_merge(db: &Database, key: &str, value: &str, hlc: &str) -> Result<bool> {
    if !is_portable_setting(key) {
        eprintln!("sync: dropping non-portable setting key {key:?} (device-local state)");
        return Ok(false);
    }
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
        assert_eq!(version, 6, "a v2 db converges through v3..=v6 to the current v6");

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
    // (body lives below the S7 push-cursor tests)

    // --- S7: destination-scoped push cursors ---

    #[test]
    fn test_push_cursor_scoped_per_destination() {
        let db = Database::open(":memory:", None).unwrap();
        // No cursor anywhere yet.
        assert_eq!(push_cursor_get(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-a"), None);
        // Distinct destinations keep independent cursors.
        push_cursor_put(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-a", "0000000000100-0000-aaaa0000")
            .unwrap();
        push_cursor_put(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-b", "0000000000200-0000-aaaa0000")
            .unwrap();
        assert_eq!(
            push_cursor_get(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-a").as_deref(),
            Some("0000000000100-0000-aaaa0000")
        );
        assert_eq!(
            push_cursor_get(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-b").as_deref(),
            Some("0000000000200-0000-aaaa0000")
        );
        // Transports don't share scopes either; a never-seen fingerprint is
        // a full push (None).
        assert_eq!(push_cursor_get(&db, IROH_PUSH_CURSOR_PREFIX, "fp-a"), None);
        // Fingerprinting is order-stable only because the CALLER normalizes;
        // distinct normalized inputs must collide never in practice.
        assert_ne!(
            destination_fingerprint(&["wss://a,wss://b", "pub1"]),
            destination_fingerprint(&["wss://a,wss://c", "pub1"])
        );
        assert_ne!(
            destination_fingerprint(&["wss://a,wss://b", "pub1"]),
            destination_fingerprint(&["wss://a,wss://b", "pub2"])
        );
        // Same input → same fingerprint (determinism).
        assert_eq!(
            destination_fingerprint(&["x", "y"]),
            destination_fingerprint(&["x", "y"])
        );
        // Length-prefixing prevents concatenation aliasing.
        assert_ne!(
            destination_fingerprint(&["ab", "c"]),
            destination_fingerprint(&["a", "bc"])
        );
    }

    #[test]
    fn test_push_cursor_legacy_migrates_once() {
        let db = Database::open(":memory:", None).unwrap();
        // Pre-S7 database: only the UNSCOPED cursor row exists.
        user_setting_upsert(&db, NOSTR_PUSH_CURSOR_PREFIX, "0000000000500-0000-aaaa0000").unwrap();
        // First read scopes the legacy cursor to the then-current
        // destination (continuity: no full resend on upgrade)...
        assert_eq!(
            push_cursor_get(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-a").as_deref(),
            Some("0000000000500-0000-aaaa0000")
        );
        assert_eq!(
            user_setting_read(&db, &push_cursor_key(NOSTR_PUSH_CURSOR_PREFIX, "fp-a"))
                .as_deref(),
            Some("0000000000500-0000-aaaa0000")
        );
        // ...but a DIFFERENT destination seen afterwards inherits nothing.
        assert_eq!(push_cursor_get(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-b"), None);
        // The legacy row is kept (not deleted).
        assert_eq!(
            user_setting_read(&db, NOSTR_PUSH_CURSOR_PREFIX).as_deref(),
            Some("0000000000500-0000-aaaa0000")
        );
        // iroh prefix migrates independently.
        assert_eq!(push_cursor_get(&db, IROH_PUSH_CURSOR_PREFIX, "fp-a"), None);
    }

    #[test]
    fn test_rewind_push_cursors_below_rewinds_only_at_or_above() {
        let db = Database::open(":memory:", None).unwrap();
        let high = "0000000002000-0000-aaaa0000";
        let low = "0000000000100-0000-aaaa0000";
        // Scoped cursors for both transports + the legacy unscoped rows.
        push_cursor_put(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-a", high).unwrap();
        push_cursor_put(&db, IROH_PUSH_CURSOR_PREFIX, "ns1", high).unwrap();
        user_setting_upsert(&db, NOSTR_PUSH_CURSOR_PREFIX, high).unwrap();
        // A cursor BELOW the merged minimum (destination already missing the
        // rows? no — it's behind, it will send them anyway): untouched.
        push_cursor_put(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-behind", low).unwrap();
        // Pull cursor must never move.
        user_setting_upsert(&db, "nostr_last_pull_hlc", high).unwrap();
        // The migration marker is not a cursor: untouched.
        user_setting_upsert(&db, &format!("{NOSTR_PUSH_CURSOR_PREFIX}@migrated"), "1").unwrap();

        let merged_min = "0000000001000-0003-bbbb0001";
        let n = rewind_push_cursors_below(&db, merged_min).unwrap();
        // nostr@fp-a, iroh@ns1, legacy nostr → 3 rewinds.
        assert_eq!(n, 3);
        for key in [
            push_cursor_key(NOSTR_PUSH_CURSOR_PREFIX, "fp-a"),
            push_cursor_key(IROH_PUSH_CURSOR_PREFIX, "ns1"),
            NOSTR_PUSH_CURSOR_PREFIX.to_string(),
        ] {
            let v = user_setting_read(&db, &key).unwrap();
            // Rewound to strictly BELOW the merged minimum, so the minimum
            // row itself is re-selected (`hlc > cursor`).
            assert!(v.as_str() < merged_min, "{key} not below min: {v}");
            // And far enough: nothing between the new cursor and merged_min
            // can exist at a DIFFERENT millisecond either.
            assert!(merged_min.parse::<Hlc>().unwrap().ms - v.parse::<Hlc>().unwrap().ms >= 1);
        }
        // Untouched rows.
        assert_eq!(
            push_cursor_get(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-behind").as_deref(),
            Some(low)
        );
        assert_eq!(user_setting_read(&db, "nostr_last_pull_hlc").as_deref(), Some(high));
        assert_eq!(
            user_setting_read(&db, &format!("{NOSTR_PUSH_CURSOR_PREFIX}@migrated")).as_deref(),
            Some("1")
        );
        // Idempotent: a second rewind with the same minimum is a no-op now.
        assert_eq!(rewind_push_cursors_below(&db, merged_min).unwrap(), 0);
        // A later import merging EVEN OLDER rows rewinds the same cursors
        // further still (cursors only move down).
        let older_min = "0000000000500-0000-cccc0002";
        let n2 = rewind_push_cursors_below(&db, older_min).unwrap();
        assert_eq!(n2, 3);
        for key in [
            push_cursor_key(NOSTR_PUSH_CURSOR_PREFIX, "fp-a"),
            push_cursor_key(IROH_PUSH_CURSOR_PREFIX, "ns1"),
            NOSTR_PUSH_CURSOR_PREFIX.to_string(),
        ] {
            let v = user_setting_read(&db, &key).unwrap();
            assert!(v.as_str() < older_min, "{key} not below older min: {v}");
        }
        // Still-behind cursor stays put (100 < 500).
        assert_eq!(
            push_cursor_get(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp-behind").as_deref(),
            Some(low)
        );
    }

    #[test]
    fn test_rewind_then_events_since_recovers_merged_history() {
        let db = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();
        // Device pushes everything up to now, then history with an OLDER
        // hlc arrives via import: selection without rewind would skip it.
        engine.submit_review(&db, "card-x", 3).unwrap();
        let cursor = Some(hlc_now(&db).to_string());
        assert_eq!(events_since(&db, cursor.as_deref()).unwrap().len(), 0);

        // Forge an old merged row (as an import would have written) and
        // rewind below it.
        let old_hlc = "0000000000001-0000-oldpeer1".to_string();
        db.conn()
            .execute(
                "INSERT INTO review_logs (uuid, card_id, timestamp, rating, elapsed_days, scheduled_days, source, device_id, hlc)
                 VALUES ('old-uuid', 'card-y', '2024-01-01T00:00:00.000Z', 3, 0.0, 0.0, 'flashcard', 'other-dev', ?1)",
                rusqlite::params![old_hlc],
            )
            .unwrap();
        assert_eq!(events_since(&db, cursor.as_deref()).unwrap().len(), 0);
        // rewind via the cursor row itself: seed a scoped cursor at `cursor`
        // and rewind below old_hlc.
        push_cursor_put(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp", &cursor.clone().unwrap()).unwrap();
        assert_eq!(rewind_push_cursors_below(&db, &old_hlc).unwrap(), 1);
        let rewound = push_cursor_get(&db, NOSTR_PUSH_CURSOR_PREFIX, "fp");
        let selected = events_since(&db, rewound.as_deref()).unwrap();
        assert!(
            selected.iter().any(|e| e.uuid == "old-uuid"),
            "merged old row must be re-selected after rewind"
        );
        // And the cursor advanced past it still excludes what was pushed.
        let after: Vec<_> = selected.iter().filter(|e| e.hlc > cursor.clone().unwrap()).collect();
        assert!(after.is_empty());
    }

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
    // incremental submit path on both sides, including source-aware practice
    // lapse policy, and the fold reproduces exactly that (verified against A).
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

    #[test]
    fn test_graduated_practice_miss_rebuild_and_shuffled_sync_preserve_memory() {
        let db_a = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();
        engine.submit_review(&db_a, "practice-lapse", 3).unwrap();
        let before: (f32, f32) = db_a
            .conn()
            .query_row(
                "SELECT stability, difficulty FROM memory_states
                 WHERE card_id = 'practice-lapse'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        engine
            .submit_review_with_source(&db_a, "practice-lapse", 1, "practice")
            .unwrap();

        let expected = memory_snapshot(&db_a);
        assert_eq!((expected[0].1, expected[0].2), (before.0 as f64, before.1 as f64));
        assert_eq!((expected[0].5, expected[0].6), (2, 1));
        let last_review = expected[0].3.as_deref().unwrap();
        let gap = crate::srs::iso_to_secs(&expected[0].4).unwrap()
            - crate::srs::iso_to_secs(last_review).unwrap();
        assert_eq!(gap, 60);

        // A direct full rebuild must be a byte-exact no-op.
        rebuild_memory_states(&db_a).unwrap();
        assert_eq!(memory_snapshot(&db_a), expected);

        let mut shuffled = events_since(&db_a, None).unwrap();
        shuffled.reverse();
        let db_b = Database::open(":memory:", None).unwrap();
        assert_eq!(apply_review_events(&db_b, &shuffled).unwrap(), 2);
        assert_eq!(memory_snapshot(&db_b), expected);

        // Applying each shuffled event separately exercises intermediate folds
        // and collision handling; replaying the complete set again is a no-op.
        let db_c = Database::open(":memory:", None).unwrap();
        for event in &shuffled {
            assert_eq!(apply_review_events(&db_c, &[event.clone()]).unwrap(), 1);
        }
        assert_eq!(memory_snapshot(&db_c), expected);
        assert_eq!(apply_review_events(&db_c, &shuffled).unwrap(), 0);
        assert_eq!(memory_snapshot(&db_c), expected);
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

    // 13. PORTABLE ALLOWLIST (unit): the exact keys the app legitimately
    //     syncs today pass; identity, HLC cursors, push/pull watermarks and
    //     local caches are refused.
    #[test]
    fn test_is_portable_setting_allowlist() {
        for portable in [
            "theme",
            "new_per_day",
            "nostr_relays",
            "alphabet_gate",
            "alphabet_gate_at",
            "share.enabled",
            "share.relays",
            "share.template_locale",
            "share.milestone.streak7.enabled",
            "share.template.streak7.en",
            "share.posted.2026-W43-streak7",
        ] {
            assert!(is_portable_setting(portable), "{portable} must sync");
        }
        for local in [
            "device_id",
            "hlc_last_ms",
            "hlc_last_counter",
            "nostr_last_push_hlc",
            "nostr_last_pull_hlc",
            "iroh_last_push_hlc",
            "share.streak.cache",
            "note",
            "",
        ] {
            assert!(!is_portable_setting(local), "{local} is device-local");
        }
    }

    // 14. PORTABLE ALLOWLIST at ingress: a remote batch (or imported backup)
    //     carrying device-local keys (identity, sync cursors) is DROPPED —
    //     the local rows stay untouched and the count of applied settings
    //     excludes the dropped rows — while a portable key in the same batch
    //     still merges.
    #[test]
    fn test_setting_merge_drops_local_only_keys() {
        let db = Database::open(":memory:", None).unwrap();
        // Pre-existing local-only state the poison batch tries to overwrite.
        setting_put(&db, "theme", "dark").unwrap();
        let local_device = device_id(&db);
        db.conn()
            .execute(
                "INSERT INTO user_settings (key, value) VALUES ('nostr_last_push_hlc', '01760000000000-0000-aabbccdd')",
                [],
            )
            .unwrap();
        // Far-future HLC: every incoming row would win the LWW race if it
        // were not dropped at ingress.
        let hostile_hlc = Hlc { ms: 4_000_000_000_000, counter: 0, node: "badpeer1".into() }.to_string();

        let kv = vec![
            ("device_id".to_string(), "00000000-foreign-device".to_string(), hostile_hlc.clone()),
            ("nostr_last_push_hlc".to_string(), "0000000000001-0000-badpeer1".to_string(), hostile_hlc.clone()),
            ("hlc_last_ms".to_string(), "1".to_string(), hostile_hlc.clone()),
            ("share.streak.cache".to_string(), "{\"streak\":9999}".to_string(), hostile_hlc.clone()),
            ("theme".to_string(), "solarized".to_string(), hostile_hlc.clone()),
        ];
        let applied = apply_settings(&db, &kv).unwrap();
        assert_eq!(applied, 1, "only the portable key may apply; drops are not counted");

        // Identity and cursors are untouched...
        assert_eq!(device_id(&db), local_device);
        assert_eq!(user_setting(&db, "nostr_last_push_hlc"), "01760000000000-0000-aabbccdd");
        // `hlc_last_ms` exists locally from legitimate writes (hlc_now), but
        // must not hold the poisoned value.
        assert_ne!(user_setting(&db, "hlc_last_ms"), "1");
        // ...and the poisoned keys never entered either settings table
        // (user_settings legitimately holds bootstrap keys like device_id /
        // hlc_last_ms, so only keys with NO legitimate local writer are
        // asserted absent there).
        for (table, keys) in [
            ("user_settings", "('share.streak.cache')"),
            ("settings_sync", "('device_id', 'nostr_last_push_hlc', 'hlc_last_ms', 'share.streak.cache')"),
        ] {
            let n: i64 = db
                .conn()
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE key IN {keys}"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{table} must never receive dropped rows");
        }
        // The portable key in the same batch merged normally.
        assert_eq!(user_setting(&db, "theme"), "solarized");
    }

    // 15. TIMESTAMP VALIDATOR (unit): only exact `YYYY-MM-DDTHH:MM:SSZ`
    //     passes — range, separator and shape violations are refused. Date
    //     correctness beyond month 1..=12 / day 1..=31 is intentionally out
    //     of scope (parse_iso_ymd's rules), so 2026-02-29 is accepted.
    #[test]
    fn test_is_canonical_timestamp() {
        for valid in [
            "2025-10-09T12:00:00Z",
            "1970-01-01T00:00:00Z",
            "2024-02-29T23:59:59Z",
            "2026-02-29T00:00:00Z", // day range 1..=31 only; no leap math here
        ] {
            assert!(is_canonical_timestamp(valid), "{valid}");
        }
        for invalid in [
            "202α-10-09T12:00:00Z",  // non-ASCII (the original probe)
            "2025-10-09T24:00:00Z",  // 24:00 not allowed without date rollover
            "2025-10-09T23:59:60Z",  // no leap seconds
            "2025-10-09T12:60:00Z",  // minute range
            "2025-13-09T12:00:00Z",  // month range
            "2025-10-00T12:00:00Z",  // day range
            "0000-10-09T12:00:00Z",  // year 0
            "2025-10-09 12:00:00Z",  // space instead of T
            "2025-10-09t12:00:00z",  // case is fixed
            "2025-10-09T12:00:00+00:00", // no offsets
            "2025-10-09T12:00:00.000Z",  // no fractional seconds
            "2025-10-09T12:00:00",   // missing Z
            "2025-10-09",            // date only
            "2025/10/09T12:00:00Z",  // wrong separators
            "2025-1-9T12:00:00Z",    // zero-padded fields only
            "",
        ] {
            assert!(!is_canonical_timestamp(invalid), "{invalid}");
        }
    }

    // 16. TIMESTAMP INGRESS: an event carrying a malformed timestamp (the
    //     original `202α-…` probe, bad time ranges) is REJECTED before any
    //     mutation — never written, never folded — while valid events in the
    //     same batch still merge; afterwards the core is fully functional
    //     (an unrelated API call succeeds on the same handle).
    #[test]
    fn test_apply_rejects_noncanonical_timestamps() {
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
        let mk = |timestamp: &str| ReviewEvent {
            uuid: uuid::Uuid::new_v4().to_string(),
            timestamp: timestamp.into(),
            ..base.clone()
        };

        // Poison-only batch: zero rows applied, nothing written.
        let probe = mk("202α-10-09T12:00:00Z");
        assert_eq!(apply_review_events(&db, &[probe]).unwrap(), 0);
        let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM review_logs", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM memory_states", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);

        // Mixed batch: bad timestamps are dropped, the valid event merges.
        let good = mk("2025-10-09T12:00:00Z");
        let inserted = apply_review_events(
            &db,
            &[
                mk("2025-10-09T24:00:00Z"),
                mk("2025-10-09T23:59:60Z"),
                mk("2025-10-09T12:00:00"), // missing Z
                good.clone(),
            ],
        )
        .unwrap();
        assert_eq!(inserted, 1);
        assert_eq!(review_uuid_set(&db), vec![good.uuid.clone()]);
        let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM memory_states", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);

        // The rejected poison did not poison the core: unrelated API calls
        // on the same handle still succeed.
        SrsEngine::new().submit_review(&db, "after-poison", 3).unwrap();
        let reps: i64 = db
            .conn()
            .query_row("SELECT reps FROM memory_states WHERE card_id = 'after-poison'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(reps, 1);
    }

    /// Read the persisted HLC clock components (test helper).
    fn stored_clock(db: &Database) -> (i64, i64) {
        let read = |key: &str| -> i64 {
            db.conn()
                .query_row(
                    "SELECT value FROM user_settings WHERE key = ?1",
                    [key],
                    |row| row.get::<_, String>(0),
                )
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        (read("hlc_last_ms"), read("hlc_last_counter"))
    }

    // 17. MONOTONICITY: observing an ahead HLC advances the stored clock;
    //     observing an older (or equal) HLC never rewinds it, and the next
    //     hlc_now stamp strictly dominates everything observed.
    #[test]
    fn test_hlc_observe_monotonic_never_rewinds() {
        let db = Database::open(":memory:", None).unwrap();
        let future = Hlc { ms: 4_000_000_000_000, counter: 3, node: "cafebabe".into() }.to_string();

        assert!(hlc_observe(&db, &future).unwrap(), "first observation must move the clock");
        assert_eq!(stored_clock(&db), (4_000_000_000_000, 3));

        // Older ms: no rewind, returns false.
        let older = Hlc { ms: 3_000_000_000_000, counter: 99, node: "cafebabe".into() }.to_string();
        assert!(!hlc_observe(&db, &older).unwrap());
        assert_eq!(stored_clock(&db), (4_000_000_000_000, 3));

        // Same ms, lower counter: no rewind.
        let lower_ctr = Hlc { ms: 4_000_000_000_000, counter: 1, node: "cafebabe".into() }.to_string();
        assert!(!hlc_observe(&db, &lower_ctr).unwrap());
        assert_eq!(stored_clock(&db), (4_000_000_000_000, 3));

        // Same ms, higher counter: counter moves, ms stays.
        let higher_ctr = Hlc { ms: 4_000_000_000_000, counter: 7, node: "feedf00d".into() }.to_string();
        assert!(hlc_observe(&db, &higher_ctr).unwrap());
        assert_eq!(stored_clock(&db), (4_000_000_000_000, 7));

        // Equal: no movement.
        assert!(!hlc_observe(&db, &higher_ctr).unwrap());
        assert_eq!(stored_clock(&db), (4_000_000_000_000, 7));

        // The next local stamp strictly dominates the observed remote
        // value even though the remote ms is far ahead of wall time —
        // and carries the LOCAL node tag, not the remote one.
        let next = hlc_now(&db);
        let observed: Hlc = higher_ctr.parse().unwrap();
        assert!(
            (next.ms, next.counter) > (observed.ms, observed.counter),
            "next stamp {next} must sort after observed {observed}"
        );
        assert_eq!(next.node, hlc_node_of(&device_id(&db)));

        // Malformed HLC is an error, not a silent no-op.
        assert!(hlc_observe(&db, "not-an-hlc").is_err());
    }

    // 18. INGRESS OBSERVATION: apply_review_events and apply_settings both
    //     advance the persisted clock to the batch max HLC (S4 receive rule).
    #[test]
    fn test_ingress_paths_observe_remote_hlc() {
        let db = Database::open(":memory:", None).unwrap();
        let ahead_ms = 3_500_000_000_000i64;
        let ev = ReviewEvent {
            uuid: uuid::Uuid::new_v4().to_string(),
            card_id: "observe-1".into(),
            timestamp: "2025-10-09T12:00:00Z".into(),
            rating: 3,
            elapsed_days: 0.0,
            scheduled_days: 1.0,
            source: "flashcard".into(),
            device_id: "deadbeefcafe".into(),
            hlc: Hlc { ms: ahead_ms, counter: 2, node: "abcd1234".into() }.to_string(),
        };
        assert_eq!(apply_review_events(&db, std::slice::from_ref(&ev)).unwrap(), 1);
        assert_eq!(stored_clock(&db), (ahead_ms, 2), "apply_review_events must observe the batch max");

        // A duplicate re-apply (inserts nothing) still repairs the clock
        // if it was behind — dup-skip must not skip observation.
        let db2 = Database::open(":memory:", None).unwrap();
        let first: Hlc = hlc_now(&db2).to_string().parse().unwrap();
        assert_eq!(apply_review_events(&db2, &[ev.clone()]).unwrap(), 1);
        // Reset the clock behind the event, then replay as a duplicate.
        db2.conn()
            .execute(
                "UPDATE user_settings SET value = ?1 WHERE key = 'hlc_last_ms'",
                [(first.ms - 1000).to_string()],
            )
            .unwrap();
        assert_eq!(apply_review_events(&db2, &[ev]).unwrap(), 0, "replay is a dup");
        assert_eq!(stored_clock(&db2).0, ahead_ms, "dup replay still observes");

        // apply_settings observes the max row HLC too.
        let db3 = Database::open(":memory:", None).unwrap();
        let kv = vec![
            ("theme".to_string(), "dark".to_string(), Hlc { ms: 2_000_000_000_000, counter: 0, node: "abcd1234".into() }.to_string()),
            ("new_per_day".to_string(), "10".to_string(), Hlc { ms: 2_500_000_000_000, counter: 5, node: "abcd1234".into() }.to_string()),
        ];
        assert_eq!(apply_settings(&db3, &kv).unwrap(), 2);
        assert_eq!(stored_clock(&db3), (2_500_000_000_000, 5), "apply_settings must observe the batch max");
    }

    // 19. HEADLINE (S4): SKEWED-CLOCK TWO-REPLICA CONVERGENCE. Replica A
    //     and replica B both apply remote reviews whose HLCs are AHEAD of
    //     their wall clocks; each then writes a local event; after
    //     exchanging full histories both fold byte-identical memory_states.
    //     Without hlc_observe the local events stamp at wall time, sort
    //     BEFORE the remote events they causally followed, and the two
    //     replicas fold the identical event set in different orders —
    //     the observed probe divergence (stability 0.949919 vs 7.861664).
    #[test]
    fn test_skewed_clock_two_replica_convergence() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let wall_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        // Remote device X (clock 2h/4h ahead of wall): two reviews on one card.
        let mk_remote = |ms: i64, ts: &str, rating: u32| ReviewEvent {
            uuid: uuid::Uuid::new_v4().to_string(),
            card_id: "skew-card".into(),
            timestamp: ts.into(),
            rating,
            elapsed_days: 0.0,
            scheduled_days: 1.0,
            source: "flashcard".into(),
            device_id: "aaaa1111-2222-3333-4444-555566667777".into(),
            hlc: Hlc { ms, counter: 0, node: "aaaa1111".into() }.to_string(),
        };
        let x1 = mk_remote(wall_ms + 2 * 3_600_000, "2026-10-19T10:00:00Z", 4);
        let x2 = mk_remote(wall_ms + 4 * 3_600_000, "2026-10-20T10:00:00Z", 1);

        let db_a = Database::open(":memory:", None).unwrap();
        let db_b = Database::open(":memory:", None).unwrap();
        assert_eq!(apply_review_events(&db_a, &[x1.clone(), x2.clone()]).unwrap(), 2);
        assert_eq!(apply_review_events(&db_b, &[x1.clone(), x2.clone()]).unwrap(), 2);

        // Each replica now writes a LOCAL event — this forces hlc_now, and
        // only the observed remote clock keeps the stamp ahead of X's HLCs.
        let engine = SrsEngine::new();
        engine.submit_review(&db_a, "skew-card", 3).unwrap();
        engine.submit_review(&db_b, "skew-card", 2).unwrap();

        let local_a = events_since(&db_a, Some(&x2.hlc)).unwrap();
        let local_b = events_since(&db_b, Some(&x2.hlc)).unwrap();
        assert_eq!(local_a.len(), 1, "A's local event must sort after X's batch");
        assert_eq!(local_b.len(), 1, "B's local event must sort after X's batch");
        assert!(local_a[0].hlc > x2.hlc, "observed remote clock must push A's stamp past it");
        assert!(local_b[0].hlc > x2.hlc, "observed remote clock must push B's stamp past it");

        // Full-history exchange both ways, then fold.
        let hist_a = events_since(&db_a, None).unwrap();
        let hist_b = events_since(&db_b, None).unwrap();
        assert_eq!(apply_review_events(&db_a, &hist_b).unwrap(), 1, "only B's local event is new to A");
        assert_eq!(apply_review_events(&db_b, &hist_a).unwrap(), 1, "only A's local event is new to B");
        rebuild_memory_states(&db_a).unwrap();
        rebuild_memory_states(&db_b).unwrap();

        let snap_a = memory_snapshot(&db_a);
        let snap_b = memory_snapshot(&db_b);
        assert_eq!(snap_a, snap_b, "identical event sets MUST fold to byte-identical memory_states");
        assert_eq!(review_uuid_set(&db_a), review_uuid_set(&db_b));
        // The headline invariant, spelled out: stability is exactly equal
        // (the S4 probe diverged: 0.949919 vs 7.861664).
        assert_eq!(snap_a.len(), 1);
        assert_eq!(snap_a[0].1, snap_b[0].1, "stability must match exactly");
        // And the causal order survived: every replica's fold sees X1, X2,
        // then the two local events (all strictly after X2's HLC).
        let order: Vec<String> = {
            let mut s = db_a
                .conn()
                .prepare("SELECT uuid FROM review_logs ORDER BY hlc, uuid")
                .unwrap();
            s.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        let causal: Vec<String> = [x1.uuid.clone(), x2.uuid.clone()]
            .into_iter()
            .chain([local_a[0].uuid.clone(), local_b[0].uuid.clone()])
            .collect();
        // The local events' HLCs can tie on (ms, counter) across devices;
        // SQL breaks the tie on the full hlc string (node), then uuid —
        // sort expected pairs the same way.
        let mut local_pairs = [
            (local_a[0].hlc.clone(), local_a[0].uuid.clone()),
            (local_b[0].hlc.clone(), local_b[0].uuid.clone()),
        ];
        local_pairs.sort();
        let expected_tail: Vec<String> = local_pairs.iter().map(|(_, u)| u.clone()).collect();
        assert_eq!(order[..2], causal[..2], "remote events lead in every fold");
        assert_eq!(order[2..], expected_tail[..], "local events follow in every fold");
    }

    // 20. FOLD-MIRROR REGRESSION (with the C6 ladder): a card whose FINAL
    //     event is an in-learning Again (reps == lapses throughout) must
    //     converge identically between the incremental submit path and a
    //     full rebuild_memory_states fold — step_due gives Again 60s for
    //     in-learning cards, so due_date must be exactly ts + 60s on both.
    #[test]
    fn test_fold_mirror_final_in_learning_again() {
        let db_a = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();
        // Nothing but Again reviews: reps == lapses at every step, so the
        // card stays in (re)learning and the final event takes the 60s step.
        engine.submit_review(&db_a, "ladder-card", 1).unwrap();
        engine.submit_review(&db_a, "ladder-card", 1).unwrap();
        engine.submit_review(&db_a, "ladder-card", 1).unwrap();

        let events = events_since(&db_a, None).unwrap();
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|e| e.rating == 1));

        // Full-fold replica.
        let db_b = Database::open(":memory:", None).unwrap();
        assert_eq!(apply_review_events(&db_b, &events).unwrap(), 3);
        // And an explicit standalone rebuild on A for good measure.
        rebuild_memory_states(&db_a).unwrap();

        let snap_a = memory_snapshot(&db_a);
        let snap_b = memory_snapshot(&db_b);
        assert_eq!(snap_a, snap_b, "incremental submit and full fold must converge");
        let (_, stability, _, last_review, due_date, reps, lapses) = &snap_a[0];
        assert_eq!((*reps, *lapses), (3, 3));
        let lr = last_review.as_deref().expect("last_review recorded");
        let gap = crate::srs::iso_to_secs(due_date).unwrap()
            - crate::srs::iso_to_secs(lr).unwrap();
        assert_eq!(gap, 60, "in-learning Again must land on the 60s step: due={due_date} last={lr}");
        assert!(*stability > 0.0);
    }

    // 21. S1 CARD-ID GATE (unit): Unicode lookup ids pass and NFC-normalize;
    //     empty, overlong, control-char and identifier-breaking ids are
    //     refused. This gate guards BOTH file import and transport pulls
    //     (both call apply_review_events).
    #[test]
    fn test_card_id_gate_unicode_and_rejects() {
        use unicode_normalization::UnicodeNormalization;

        // The headline id: an app-created Greek lookup card.
        assert_eq!(
            normalize_card_id("lookup-λόγος").as_deref(),
            Some("lookup-λόγος")
        );
        // NFD spelling of the same lemma normalizes to the NFC wire id:
        // both devices must converge on ONE card id.
        let nfd: String = "lookup-λο\u{0301}γος".nfd().collect();
        assert_ne!(nfd, "lookup-λόγος", "fixture must be NFD");
        assert_eq!(
            normalize_card_id(&nfd).as_deref(),
            Some("lookup-λόγος"),
            "NFD ingress must canonicalize to NFC"
        );
        // Deck ids, mixed scripts, punctuation kept from the old charset.
        for ok in ["athenaze-i-0001", "letters-001", "a_b.c-日本語-χαίρε"] {
            assert_eq!(normalize_card_id(ok).as_deref(), Some(ok), "{ok}");
        }
        // Rejects.
        assert_eq!(normalize_card_id(""), None);
        let overlong = "x".repeat(CARD_ID_MAX_LEN + 1);
        assert_eq!(normalize_card_id(&overlong), None);
        assert_eq!(normalize_card_id("bad\ncard"), None, "control chars");
        assert_eq!(normalize_card_id("bad\u{7f}card"), None, "DEL");
        assert_eq!(normalize_card_id("x'; DROP TABLE memory_states;--"), None, "quote");
        assert_eq!(normalize_card_id("bad\"card"), None);
        assert_eq!(normalize_card_id("bad\\card"), None);
        // Exactly at the cap: accepted.
        let at_cap = format!("lookup-{}", "λ".repeat(CARD_ID_MAX_LEN - 7));
        assert_eq!(normalize_card_id(&at_cap).as_deref(), Some(at_cap.as_str()));
    }

    // 22. S1 INGRESS: a review event for `lookup-λόγος` (app-created, Greek)
    //     APPLIES — the old ASCII gate silently dropped it (the probe bug);
    //     an NFD spelling of the same id is stored under the NFC form so
    //     both spellings fold into ONE card.
    #[test]
    fn test_apply_accepts_and_normalizes_unicode_card_ids() {
        db_test_lookup_roundtrip_shape();
    }

    /// Shared by test 22 (kept as a fn so the flow reads linearly).
    fn db_test_lookup_roundtrip_shape() {
        let db_a = Database::open(":memory:", None).unwrap();
        let engine = SrsEngine::new();
        // The real submit path on A: app-created Greek lookup id.
        engine.submit_review(&db_a, "lookup-λόγος", 3).unwrap();
        engine.submit_review(&db_a, "lookup-λόγος", 4).unwrap();
        let events = events_since(&db_a, None).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.card_id == "lookup-λόγος"));

        // B applies the batch: the Unicode id passes the gate.
        let db_b = Database::open(":memory:", None).unwrap();
        assert_eq!(apply_review_events(&db_b, &events).unwrap(), 2);
        assert_eq!(memory_snapshot(&db_a), memory_snapshot(&db_b));

        // An incoming NFD spelling of the same id lands under the NFC id —
        // one card, three reviews, after the merge.
        let nfd_event = ReviewEvent {
            uuid: uuid::Uuid::new_v4().to_string(),
            card_id: "lookup-λο\u{0301}γος".into(), // NFD
            timestamp: "2026-10-20T12:00:00Z".into(),
            rating: 3,
            elapsed_days: 1.0,
            scheduled_days: 1.0,
            source: "flashcard".into(),
            device_id: "otherdevice".into(),
            hlc: Hlc { ms: 1_800_000_000_000, counter: 0, node: "cafebabe".into() }.to_string(),
        };
        assert_eq!(apply_review_events(&db_b, std::slice::from_ref(&nfd_event)).unwrap(), 1);
        let stored: Vec<String> = {
            let mut s = db_b
                .conn()
                .prepare("SELECT DISTINCT card_id FROM review_logs")
                .unwrap();
            s.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<std::result::Result<Vec<String>, _>>()
                .unwrap()
        };
        assert_eq!(stored, vec!["lookup-λόγος".to_string()], "NFC convergence");
        let reps: i64 = db_b
            .conn()
            .query_row("SELECT reps FROM memory_states WHERE card_id = 'lookup-λόγος'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(reps, 3);
    }

    // 23. VERSION POLICY (unit): v1, missing-v (defaults to 1) and v2
    //     batches pass; v0 / v99 are rejected by the shared gate; a v1
    //     JSON document parses with EMPTY v2 collections.
    #[test]
    fn test_validate_batch_version_policy() {
        let mk = |v: u32| SyncBatch {
            v,
            device_id: "dev".into(),
            from_hlc: String::new(),
            to_hlc: "1762000000000-0000-feedface".into(),
            reviews: vec![],
            settings: vec![],
            known_words: vec![],
            lookup_cards: vec![],
            chapter_unlocks: vec![],
        };
        validate_batch(&mk(1)).unwrap();
        validate_batch(&mk(2)).unwrap();
        let err = validate_batch(&mk(0)).unwrap_err();
        assert!(err.to_string().contains("unsupported sync wire version"), "{err}");
        let err = validate_batch(&mk(99)).unwrap_err();
        assert!(err.to_string().contains("unsupported sync wire version"), "{err}");

        // v1 document shape (no v2 keys, explicit "v": 1): parses with
        // empty content collections.
        let v1_json = r#"{"v":1,"device_id":"dev","from_hlc":"","to_hlc":"1762000000000-0000-feedface","reviews":[],"settings":[["theme","dark","1762000000000-0000-feedface"]]}"#;
        let batch: SyncBatch = serde_json::from_str(v1_json).unwrap();
        assert_eq!(batch.v, 1);
        assert!(batch.known_words.is_empty() && batch.lookup_cards.is_empty() && batch.chapter_unlocks.is_empty());
        validate_batch(&batch).unwrap();

        // A pre-versioning document (no "v" key at all) reads as v1.
        let nov_json = r#"{"device_id":"dev","from_hlc":"","to_hlc":"1762000000000-0000-feedface","reviews":[],"settings":[]}"#;
        let batch: SyncBatch = serde_json::from_str(nov_json).unwrap();
        assert_eq!(batch.v, 1);
        validate_batch(&batch).unwrap();
    }

    // 24. V2 COLLECTIONS round-trip and merge rules: known-words LWW,
    //     lookup-cards PK-dedup, chapter-unlocks grow-only — including
    //     skip-and-count ingress on junk rows.
    #[test]
    fn test_v2_collections_apply_and_dedup() {
        let db = Database::open(":memory:", None).unwrap();

        // known_words: apply, dedupe, LWW.
        let kw = vec![
            KnownWordEntry { word: "λόγος".into(), known: true, updated_at: "2026-10-20T10:00:00Z".into() },
            KnownWordEntry { word: "δοῦλος".into(), known: false, updated_at: "2026-10-20T10:00:00Z".into() },
            // junk: bad timestamp, bad word
            KnownWordEntry { word: "junk-ts".into(), known: true, updated_at: "not-a-time".into() },
            KnownWordEntry { word: "bad\nword".into(), known: true, updated_at: "2026-10-20T10:00:00Z".into() },
        ];
        assert_eq!(apply_known_words(&db, &kw).unwrap(), 2);
        assert_eq!(known_words_all(&db).unwrap().len(), 2);

        // Older write loses; newer wins.
        let older = vec![KnownWordEntry { word: "λόγος".into(), known: false, updated_at: "2026-10-19T10:00:00Z".into() }];
        assert_eq!(apply_known_words(&db, &older).unwrap(), 0);
        let newer = vec![KnownWordEntry { word: "λόγος".into(), known: false, updated_at: "2026-10-21T10:00:00Z".into() }];
        assert_eq!(apply_known_words(&db, &newer).unwrap(), 1);
        let got = known_words_all(&db).unwrap();
        let logos = got.iter().find(|e| e.word == "λόγος").unwrap();
        assert!(!logos.known && logos.updated_at == "2026-10-21T10:00:00Z");

        // lookup_cards: apply + dedup, junk dropped, id NFC-normalized.
        let lc = vec![
            LookupCardEntry { card_id: "lookup-λόγος".into(), front: "λόγος".into(), back: "word".into() },
            LookupCardEntry { card_id: "x'; DROP--".into(), front: "x".into(), back: "y".into() },
        ];
        assert_eq!(apply_lookup_cards(&db, &lc).unwrap(), 1);
        assert_eq!(apply_lookup_cards(&db, &lc).unwrap(), 0, "PK dedup");
        let nfd = vec![LookupCardEntry {
            card_id: "lookup-λο\u{0301}γος".into(), // NFD == existing NFC id
            front: "changed".into(),
            back: "changed".into(),
        }];
        assert_eq!(apply_lookup_cards(&db, &nfd).unwrap(), 0, "NFD dedups against the NFC row");
        let cards = lookup_cards_all(&db).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].front, "λόγος", "first writer wins");

        // chapter_unlocks: apply + dedup, junk dropped.
        let cu = vec![
            ChapterUnlock { textbook: "letters".into(), chapter: 2 },
            ChapterUnlock { textbook: "athenaze".into(), chapter: 5 },
            ChapterUnlock { textbook: "".into(), chapter: 1 },
            ChapterUnlock { textbook: "athenaze".into(), chapter: -1 },
        ];
        assert_eq!(apply_chapter_unlocks(&db, &cu).unwrap(), 2);
        assert_eq!(apply_chapter_unlocks(&db, &cu).unwrap(), 0, "grow-only dedup");
        let got = chapter_unlocks_all(&db).unwrap();
        assert_eq!(
            got,
            vec![
                ChapterUnlock { textbook: "athenaze".into(), chapter: 5 },
                ChapterUnlock { textbook: "letters".into(), chapter: 2 },
            ]
        );
    }

    // --- S6: applied-batch ledger (nostr pull admission) ---

    #[test]
    fn test_s6_batch_ledger_roundtrip() {
        let db = Database::open(":memory:", None).unwrap();
        assert!(!batch_is_applied(&db, "batch-a"));
        mark_batch_applied(&db, "batch-a").unwrap();
        assert!(batch_is_applied(&db, "batch-a"));
        assert!(!batch_is_applied(&db, "batch-b"), "other batches stay unadmitted");
        // Idempotent: re-marking an applied batch must not error or duplicate.
        mark_batch_applied(&db, "batch-a").unwrap();
        let n: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM sync_applied_batches", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    /// S6 headline (audit repro, inverted): complete newer batch B applies
    /// and marks FIRST; older complete batch A — whose HLCs sit BELOW the
    /// pull high-water mark — arrives later. Admission is the batch UUID,
    /// not the HLC cursor, so A still merges, and the receiver's
    /// memory_states equal a single-shot authoritative replay of A ∪ B.
    /// A re-pull of both marked batches is a clean no-op.
    #[test]
    fn test_s6_late_below_watermark_batch_still_merges() {
        let event = |uuid: &str, card: &str, ts: &str, rating: u32, hlc: &str, dev: &str| {
            ReviewEvent {
                uuid: uuid.into(),
                card_id: card.into(),
                timestamp: ts.into(),
                rating,
                elapsed_days: 0.0,
                scheduled_days: 1.0,
                source: "review".into(),
                device_id: dev.into(),
                hlc: hlc.into(),
            }
        };
        // A: two OLD reviews (HLCs below everything in B).
        let a_events = vec![
            event("a-uuid-1", "card-a1", "2026-10-01T10:00:00Z", 3, "1700000000000-0000-0badf00a", "dev-a"),
            event("a-uuid-2", "card-a2", "2026-10-01T11:00:00Z", 4, "1700000001000-0000-0badf00a", "dev-a"),
        ];
        // B: one NEWER review; its batch to_hlc advanced the high-water mark.
        let b_events = vec![
            event("b-uuid-1", "card-b1", "2026-10-05T12:00:00Z", 2, "1800000000000-0000-0badf00b", "dev-b"),
        ];
        let a_to_hlc = "1700000001000-0000-0badf00a";
        let b_to_hlc = "1800000000000-0000-0badf00b";
        assert!(a_to_hlc < b_to_hlc, "A must sort below B (the audit trap)");

        // Authoritative replay: everything in ONE apply on a fresh db.
        let auth = Database::open(":memory:", None).unwrap();
        let mut all = a_events.clone();
        all.extend(b_events.iter().cloned());
        apply_review_events(&auth, &all).unwrap();

        // Receiver: B lands first and is marked; the observability pull
        // high-water mark advances past every HLC in A.
        let rx = Database::open(":memory:", None).unwrap();
        assert!(!batch_is_applied(&rx, "batch-b"));
        rx.with_tx(|_c| -> anyhow::Result<()> {
            apply_review_events(&rx, &b_events)?;
            hlc_observe(&rx, b_to_hlc)?;
            mark_batch_applied(&rx, "batch-b")?;
            Ok(())
        })
        .unwrap();
        assert!(batch_is_applied(&rx, "batch-b"));
        assert_eq!(review_uuid_set(&rx), vec!["b-uuid-1".to_string()]);

        // A arrives LATE (previously incomplete at B's pull time, or a
        // lagging-clock device finally uploading). Its UUID is unmarked, so
        // admission accepts it DESPITE its HLCs sitting below the mark.
        assert!(!batch_is_applied(&rx, "batch-a"));
        rx.with_tx(|_c| -> anyhow::Result<()> {
            apply_review_events(&rx, &a_events)?;
            hlc_observe(&rx, a_to_hlc)?;
            mark_batch_applied(&rx, "batch-a")?;
            Ok(())
        })
        .unwrap();

        // BOTH batches merged; the fold equals the authoritative replay.
        assert_eq!(
            review_uuid_set(&rx),
            review_uuid_set(&auth),
            "receiver must hold exactly A ∪ B"
        );
        assert_eq!(
            memory_snapshot(&rx),
            memory_snapshot(&auth),
            "memory_states must equal the authoritative replay"
        );

        // Re-pull: both UUIDs marked -> admission skips -> no double apply.
        assert!(batch_is_applied(&rx, "batch-b") && batch_is_applied(&rx, "batch-a"));
        assert_eq!(
            apply_review_events(&rx, &all).unwrap(),
            0,
            "uuid dedup is the belt under the ledger suspenders"
        );
        assert_eq!(memory_snapshot(&rx), memory_snapshot(&auth));

        // Mark rides the SAME tx as application: a failed apply must leave
        // the batch unmarked and every insert rolled back.
        let bad = vec![event("x-bad-hlc", "card-x", "2026-10-06T00:00:00Z", 3, "garbage", "dev-x")];
        // (Malformed event HLC: screened out pre-tx, 0 applied. To exercise
        // the ROLLBACK path instead, poison via hlc_observe's to_hlc.)
        assert_eq!(apply_review_events(&rx, &bad).unwrap(), 0);
        let rolled_back = rx.with_tx(|_c| -> anyhow::Result<()> {
            apply_review_events(&rx, &a_events)?; // replays fine (0 inserted)
            hlc_observe(&rx, "not-an-hlc")?; // ERROR -> whole tx rolls back
            mark_batch_applied(&rx, "batch-z")?;
            Ok(())
        });
        assert!(rolled_back.is_err(), "the tx must fail");
        assert!(!batch_is_applied(&rx, "batch-z"), "no mark may survive a rolled-back apply");
        assert_eq!(memory_snapshot(&rx), memory_snapshot(&auth), "state untouched");
    }
}
