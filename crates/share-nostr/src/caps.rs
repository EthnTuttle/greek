//! Hard posting caps, derived from the synced `share.posted.*` LWW settings
//! markers plus the local outbox (design §6; Appendix R multi-device budget):
//!
//! - ≤ 2 posts per UTC day globally when this is the only known device;
//! - ≥ 6 hours between posts;
//! - with > 1 known device, this device's budget drops to 1 post/day
//!   (markers sync, so the wallet of devices still converges near ≤ 2/day
//!   total without a global lock).
//!
//! Seeded enable-time markers (event_id empty) never consume budget — their
//! whole purpose is to suppress retroactive posts, not to throttle new ones.

use crate::errors::Result;

/// Global daily cap for a single-device wallet.
pub const GLOBAL_DAILY_CAP: u32 = 2;
/// Per-device daily cap once more than one device is known.
pub const PER_DEVICE_DAILY_CAP: u32 = 1;
/// Minimum spacing between two published posts.
pub const MIN_SPACING_MS: i64 = 6 * 3600 * 1000;

/// What a successful (or, at enable-time, already-crossed) milestone leaves
/// behind in `settings_sync` under `share.posted.<dedupe_key>`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PostedMarker {
    /// The posted event id (hex). Empty string for enable-time seed markers,
    /// which represent "already crossed before sharing was enabled".
    pub event_id: String,
    pub posted_ms: i64,
    /// Relay URLs that accepted the event (csv-style list as a vec). Empty
    /// for seed markers and for read-back-detected deliveries we could not
    /// attribute.
    pub relays: Vec<String>,
}

impl PostedMarker {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("PostedMarker serialization is infallible")
    }

    /// Tolerant parse: a non-JSON value is treated as a bare event id
    /// (forward-compat for markers written by other tooling).
    pub fn from_value(value: &str) -> Self {
        serde_json::from_str(value).unwrap_or_else(|_| PostedMarker {
            event_id: value.trim().to_string(),
            posted_ms: 0,
            relays: Vec::new(),
        })
    }

    /// Seed markers (enable-time, `event_id == ""`) are not deliveries and
    /// never count toward caps.
    pub fn counts_toward_caps(&self) -> bool {
        !self.event_id.is_empty()
    }
}

pub fn marker_key(dedupe_key: &str) -> String {
    format!("share.posted.{dedupe_key}")
}

/// Write the synced LWW marker for a delivered milestone.
pub fn write_marker(
    db: &greek_core::Database,
    dedupe_key: &str,
    event_id: &str,
    relays: &[String],
    posted_ms: i64,
) -> Result<()> {
    let marker = PostedMarker {
        event_id: event_id.to_string(),
        posted_ms,
        relays: relays.to_vec(),
    };
    greek_core::sync::setting_put(db, &marker_key(dedupe_key), &marker.to_json())?;
    Ok(())
}

/// Read a posted marker, if any.
pub fn read_marker(db: &greek_core::Database, dedupe_key: &str) -> Result<Option<PostedMarker>> {
    let value: Option<String> = db
        .conn()
        .query_row(
            "SELECT value FROM user_settings WHERE key = ?1",
            [marker_key(dedupe_key)],
            |row| row.get(0),
        )
        .ok();
    Ok(value.map(|v| PostedMarker::from_value(&v)))
}

/// True when this milestone is already recorded as delivered — either by a
/// synced marker (any device) or by a row in the local outbox (any state, so
/// even a terminal failed/skipped row suppresses re-posting of that exact
/// milestone while staying in history).
pub fn already_recorded(db: &greek_core::Database, dedupe_key: &str) -> Result<bool> {
    if read_marker(db, dedupe_key)?.is_some() {
        return Ok(true);
    }
    Ok(crate::outbox::get(db, dedupe_key)?.is_some())
}

/// All deliveries with real event ids, newest-unknown-order neutral:
/// `(dedupe_key, posted_ms)` pairs used by the cap math. Merges synced
/// markers with local posted outbox rows (outbox rows lacking markers use
/// their created_ms as a lower bound — marker writes happen at post time, so
/// a missing marker means a crash between the two writes).
fn deliveries(db: &greek_core::Database) -> Result<Vec<(String, i64)>> {
    let mut by_key: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();

    let mut stmt = db.conn().prepare(
        "SELECT key, value FROM user_settings WHERE key LIKE 'share.posted.%' ESCAPE '\\'",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    for row in rows {
        let (key, value) = row?;
        let marker = PostedMarker::from_value(&value);
        if marker.counts_toward_caps() {
            let dedupe = key.trim_start_matches("share.posted.").to_string();
            by_key.insert(dedupe, marker.posted_ms);
        }
    }

    for row in crate::outbox::history(db)? {
        if row.state == crate::outbox::OutboxState::Posted
            && !by_key.contains_key(&row.dedupe_key)
        {
            by_key.insert(row.dedupe_key, row.created_ms);
        }
    }
    Ok(by_key.into_iter().collect())
}

/// Number of devices this wallet knows about: itself plus paired peers
/// (`sync_devices`). Never less than 1.
pub fn known_device_count(db: &greek_core::Database) -> u32 {
    let peers: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM sync_devices", [], |row| row.get(0))
        .unwrap_or(0);
    peers as u32 + 1
}

/// This device's daily budget.
pub fn daily_cap(db: &greek_core::Database) -> u32 {
    if known_device_count(db) > 1 {
        PER_DEVICE_DAILY_CAP
    } else {
        GLOBAL_DAILY_CAP
    }
}

/// Outcome of a cap check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapStatus {
    /// A post may go out right now.
    Allowed,
    /// The daily budget for this device is already spent.
    DailyExhausted { used: u32, cap: u32 },
    /// Within budget, but the minimum spacing hasn't elapsed yet.
    TooSoon { retry_after_ms: i64 },
}

/// Evaluate whether flush may post right now. `now_ms` is epoch millis;
/// days are UTC days (ms / 86400000).
pub fn check_caps(db: &greek_core::Database, now_ms: i64) -> Result<CapStatus> {
    let all = deliveries(db)?;
    let today = now_ms.div_euclid(86_400_000);
    let delivered_today = all
        .iter()
        .filter(|(_, ms)| ms.div_euclid(86_400_000) == today)
        .count() as u32;
    let cap = daily_cap(db);
    if delivered_today >= cap {
        return Ok(CapStatus::DailyExhausted {
            used: delivered_today,
            cap,
        });
    }
    let last_ms = all.iter().map(|(_, ms)| *ms).max();
    if let Some(last) = last_ms {
        let next_ok = last + MIN_SPACING_MS;
        if now_ms < next_ok {
            return Ok(CapStatus::TooSoon {
                retry_after_ms: next_ok - now_ms,
            });
        }
    }
    Ok(CapStatus::Allowed)
}

/// Pure variant operating on an explicit delivery list — the math the
/// sqlite-backed [`check_caps`] delegates to; kept public for unit tests and
/// for W9-C previews.
pub fn cap_math(
    delivered_ms: &[i64],
    device_count: u32,
    now_ms: i64,
) -> CapStatus {
    let cap = if device_count > 1 {
        PER_DEVICE_DAILY_CAP
    } else {
        GLOBAL_DAILY_CAP
    };
    let today = now_ms.div_euclid(86_400_000);
    let used = delivered_ms
        .iter()
        .filter(|ms| ms.div_euclid(86_400_000) == today)
        .count() as u32;
    if used >= cap {
        return CapStatus::DailyExhausted { used, cap };
    }
    if let Some(last) = delivered_ms.iter().max() {
        let next_ok = last + MIN_SPACING_MS;
        if now_ms < next_ok {
            return CapStatus::TooSoon {
                retry_after_ms: next_ok - now_ms,
            };
        }
    }
    CapStatus::Allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use greek_core::Database;

    const HOUR_MS: i64 = 3_600_000;
    const DAY_MS: i64 = 86_400_000;
    /// 1_760_000_000_000 ms is day 20370 (UTC), ~8.9h after the day boundary.
    const T0: i64 = 1_760_000_000_000;

    #[test]
    fn solo_device_cap_math() {
        // No deliveries yet: allowed.
        assert_eq!(cap_math(&[], 1, T0), CapStatus::Allowed);
        // One delivery 7h ago: spacing satisfied, one budget slot left.
        assert_eq!(cap_math(&[T0 - 7 * HOUR_MS], 1, T0), CapStatus::Allowed);
        // One delivery 5h ago: spacing blocks with exact retry delay.
        assert_eq!(
            cap_math(&[T0 - 5 * HOUR_MS], 1, T0),
            CapStatus::TooSoon {
                retry_after_ms: HOUR_MS
            }
        );
        // Two deliveries today: daily budget spent regardless of spacing.
        assert_eq!(
            cap_math(&[T0, T0 + 30_000], 1, T0 + 7 * HOUR_MS),
            CapStatus::DailyExhausted { used: 2, cap: 2 }
        );
        // A delivery YESTERDAY does not consume today's budget (but spacing
        // still applies).
        assert_eq!(
            cap_math(&[T0 - DAY_MS, T0 - 7 * HOUR_MS], 1, T0),
            CapStatus::Allowed
        );
        assert_eq!(
            cap_math(&[T0 - 6 * HOUR_MS], 1, T0),
            CapStatus::Allowed,
            "exactly 6h spacing is satisfied"
        );
    }

    #[test]
    fn multi_device_cap_math() {
        // >1 known device ⇒ per-device budget is one post/day.
        assert_eq!(
            cap_math(&[T0], 2, T0 + 7 * HOUR_MS),
            CapStatus::DailyExhausted { used: 1, cap: 1 }
        );
        assert_eq!(cap_math(&[], 2, T0), CapStatus::Allowed);
        assert_eq!(
            cap_math(&[], 5, T0),
            CapStatus::Allowed,
            "fresh device with many peers can still post one"
        );
    }

    #[test]
    fn db_caps_use_markers_and_outbox_union() {
        let db = Database::open(":memory:", None).unwrap();
        assert_eq!(known_device_count(&db), 1);
        assert_eq!(check_caps(&db, T0).unwrap(), CapStatus::Allowed);

        write_marker(&db, "m1", "deadbeef", &[], T0 - 7 * HOUR_MS).unwrap();
        // Outbox row posted without a marker (crash-window simulation) also
        // counts toward the daily cap.
        let keys = crate::identity::ShareKeys::derive_from_sync(&[0xCD; 32]).unwrap();
        let ev =
            serde_json::to_string(&keys.sign_note("x").unwrap()).unwrap();
        crate::outbox::enqueue(&db, "m2", "streak.7", &ev, "v", T0).unwrap();
        crate::outbox::set_state(&db, "m2", crate::outbox::OutboxState::Posted, None).unwrap();

        assert_eq!(
            check_caps(&db, T0 + 30_000).unwrap(),
            CapStatus::DailyExhausted { used: 2, cap: 2 }
        );
        // A day later: budget refreshes.
        assert_eq!(check_caps(&db, T0 + DAY_MS).unwrap(), CapStatus::Allowed);
    }

    #[test]
    fn pairing_a_peer_flips_the_cap_to_one_per_device() {
        let db = Database::open(":memory:", None).unwrap();
        write_marker(&db, "m1", "aa11", &[], T0).unwrap();
        // Solo: one delivery today, spacing ok → allowed.
        assert_eq!(check_caps(&db, T0 + 7 * HOUR_MS).unwrap(), CapStatus::Allowed);
        greek_core::sync::pair_device(&db, "peer-device-1", "phone").unwrap();
        assert_eq!(known_device_count(&db), 2);
        assert_eq!(
            check_caps(&db, T0 + 7 * HOUR_MS).unwrap(),
            CapStatus::DailyExhausted { used: 1, cap: 1 }
        );
    }

    #[test]
    fn seed_markers_suppress_repost_but_never_consume_budget() {
        let db = Database::open(":memory:", None).unwrap();
        // Enable-time seed: empty event_id.
        write_marker(&db, "gate.passed.check:gate", "", &[], T0).unwrap();
        assert!(already_recorded(&db, "gate.passed.check:gate").unwrap());
        // ...but the cap is untouched.
        assert_eq!(check_caps(&db, T0).unwrap(), CapStatus::Allowed);

        // A real marker both suppresses and consumes.
        write_marker(&db, "streak.7:2026-10-19", "cafe", &["wss://r".into()], T0).unwrap();
        assert!(already_recorded(&db, "streak.7:2026-10-19").unwrap());
        assert_eq!(
            check_caps(&db, T0 + 30_000).unwrap(),
            CapStatus::TooSoon {
                retry_after_ms: MIN_SPACING_MS - 30_000
            }
        );

        // An outbox row alone (no marker) also suppresses re-enqueue.
        let keys = crate::identity::ShareKeys::derive_from_sync(&[0xEF; 32]).unwrap();
        let ev = serde_json::to_string(&keys.sign_note("x").unwrap()).unwrap();
        crate::outbox::enqueue(&db, "chapter.mastered:athenaze/3", "chapter.mastered", &ev, "v", T0)
            .unwrap();
        assert!(already_recorded(&db, "chapter.mastered:athenaze/3").unwrap());
    }

    #[test]
    fn marker_json_roundtrip_and_tolerant_parse() {
        let m = PostedMarker {
            event_id: "ab".into(),
            posted_ms: T0,
            relays: vec!["wss://a".into(), "wss://b".into()],
        };
        assert_eq!(PostedMarker::from_value(&m.to_json()), m);
        // Bare non-JSON value tolerated as a raw event id.
        let bare = PostedMarker::from_value("0123abcd");
        assert_eq!(bare.event_id, "0123abcd");
        assert!(bare.counts_toward_caps());
        assert!(!PostedMarker {
            event_id: String::new(),
            posted_ms: T0,
            relays: vec![],
        }
        .counts_toward_caps());
    }
}
