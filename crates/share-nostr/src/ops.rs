//! Orchestration: enable/disable, milestone enqueue, and the flush that
//! actually publishes. Flush scheduling itself (the 5–15 minute jitter) is
//! the CALLER's job — this module only processes what is due, respecting
//! caps, exactly-once semantics, and the outbox state machine.

use crate::caps::{self, CapStatus};
use crate::errors::{Result, ShareError};
use crate::identity::ShareKeys;
use crate::outbox::{self, OutboxState};
use crate::publisher::{OkClass, PublishResult, Publisher};
use crate::templates::{self, TemplateLang};

/// The settings key (HLC-LWW-synced) gating ALL sharing network activity.
pub const ENABLED_KEY: &str = "share.enabled";

pub fn is_enabled(db: &greek_core::Database) -> bool {
    db.conn()
        .query_row(
            "SELECT value FROM user_settings WHERE key = ?1",
            [ENABLED_KEY],
            |row| row.get::<_, String>(0),
        )
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Outcome of [`enable`]: how many already-crossed milestones were seeded as
/// posted markers (and therefore suppressed from ever posting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnableReport {
    pub seeded: u32,
}

/// Opt-in (default off, never retroactive). BEFORE flipping `share.enabled`,
/// seed `share.posted.*` markers (with empty event ids — they are history,
/// not deliveries, and consume no cap budget) for EVERY currently-crossed
/// threshold. Idempotent: re-enabling only seeds what's missing.
pub fn enable(
    db: &greek_core::Database,
    cards: &[greek_core::srs::Card],
    today: &str,
) -> Result<EnableReport> {
    let now_ms = now_epoch_ms();
    let mut seeded = 0u32;

    // Gate latch (lane: check / organic only).
    if let Some(slug) = crate::detect::detect_gate_latch(db) {
        let key = format!("{slug}:gate");
        if !caps::already_recorded(db, &key)? {
            caps::write_marker(db, &key, "", &[], now_ms)?;
            seeded += 1;
        }
    }

    // Chapters already at 100% mastery.
    for ch in crate::detect::detect_chapter_mastered(db, cards)? {
        if !caps::already_recorded(db, &ch.dedupe_key)? {
            caps::write_marker(db, &ch.dedupe_key, "", &[], now_ms)?;
            seeded += 1;
        }
    }

    // Streak thresholds the current run has already crossed.
    for st in crate::detect::detect_streak_crossings(db, today)? {
        if !caps::already_recorded(db, &st.dedupe_key)? {
            caps::write_marker(db, &st.dedupe_key, "", &[], now_ms)?;
            seeded += 1;
        }
    }

    greek_core::sync::setting_put(db, ENABLED_KEY, "1")?;
    Ok(EnableReport { seeded })
}

/// Opt-out: stops the network path and clears still-live outbox rows
/// (`queued`/`posting`). History rows (posted/skipped/failed) are kept —
/// the per-slug history UI survives disable. Returns rows cleared.
pub fn disable(db: &greek_core::Database) -> Result<u32> {
    greek_core::sync::setting_put(db, ENABLED_KEY, "0")?;
    outbox::init_schema(db)?;
    let cleared = db.conn().execute(
        "DELETE FROM share_outbox WHERE state IN ('queued', 'posting')",
        [],
    )?;
    Ok(cleared as u32)
}

fn now_epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// What an enqueue attempt decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Enqueued,
    /// Marker or outbox row already exists for this milestone.
    AlreadyRecorded,
    /// Today's device budget was already spent at enqueue time; the
    /// milestone is recorded as a terminal `skipped_cap` history row.
    SkippedCap,
    /// Sharing is off — nothing was enqueued.
    Disabled,
}

/// Milestone-anchored enqueue: render (validating) the template + optional
/// custom suffix, sign a kind-1 note with the sharing key, and persist the
/// fully-signed event bytes in the outbox. Publish-time validation in
/// [`flush`] re-checks the same content on the way out.
#[allow(clippy::too_many_arguments)]
pub fn enqueue_milestone(
    db: &greek_core::Database,
    slug: &str,
    dedupe_key: &str,
    visible_id: &str,
    lang: TemplateLang,
    count: Option<u32>,
    suffix: Option<&str>,
    keys: &ShareKeys,
    now_ms: i64,
) -> Result<EnqueueOutcome> {
    if !is_enabled(db) {
        return Ok(EnqueueOutcome::Disabled);
    }
    if caps::already_recorded(db, dedupe_key)? {
        return Ok(EnqueueOutcome::AlreadyRecorded);
    }
    let content = templates::render_content(db, slug, lang, count, suffix)?;
    let event = keys.sign_note(&content)?;
    let event_json = serde_json::to_string(&event).map_err(ShareError::Json)?;

    match caps::check_caps(db, now_ms)? {
        CapStatus::DailyExhausted { .. } => {
            outbox::enqueue_skipped_cap(db, dedupe_key, slug, &event_json, visible_id, now_ms)?;
            Ok(EnqueueOutcome::SkippedCap)
        }
        // Spacing never blocks enqueueing — the flush defers within the day.
        CapStatus::TooSoon { .. } | CapStatus::Allowed => {
            outbox::enqueue(db, dedupe_key, slug, &event_json, visible_id, now_ms)?;
            Ok(EnqueueOutcome::Enqueued)
        }
    }
}

/// Aggregate outcome of one [`flush`] batch (Appendix R honesty shape).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlushReport {
    /// Rows newly posted this flush.
    pub posted: u32,
    /// Rows still queued (rate-limited / spacing / transient relay errors).
    pub deferred: u32,
    /// Rows recognized as already delivered without a fresh acceptance —
    /// relay `duplicate:` responses and pre-publish read-back hits.
    pub duplicates: u32,
    /// `(relay, reason)` for every non-delivered relay outcome, plus
    /// ("local", reason) entries for rows that failed before any network I/O
    /// (e.g. publish-time validation rejects corrupted content).
    pub failed: Vec<(String, String)>,
}

/// Process due outbox rows: expiry sweep, per-row caps, write-ahead
/// `posting` marker, pre-publish read-back, per-relay publish + OK-prefix
/// classification, marker/outbox updates. Shares NOTHING with the sync
/// client; refuses ALL network activity while disabled.
pub async fn flush(
    db: &greek_core::Database,
    publisher: &Publisher,
    now_ms: i64,
) -> Result<FlushReport> {
    let mut report = FlushReport::default();
    if !is_enabled(db) {
        return Ok(report); // zero network activity while disabled
    }
    outbox::expire_stale(db, now_ms)?;
    let rows = outbox::due_rows(db)?;

    for row in &rows {
        // --- Caps -------------------------------------------------------
        match caps::check_caps(db, now_ms)? {
            CapStatus::DailyExhausted { .. } => {
                // Day is full (see Caps docs): record as deliberate overflow.
                outbox::set_state(db, &row.dedupe_key, OutboxState::SkippedCap, None)?;
                continue;
            }
            CapStatus::TooSoon { retry_after_ms } => {
                // Spacing blocks this row AND every later row; defer them.
                let _ = retry_after_ms;
                report.deferred += rows
                    .iter()
                    .position(|r| r.dedupe_key == row.dedupe_key)
                    .map(|i| (rows.len() - i) as u32)
                    .unwrap_or(1);
                break;
            }
            CapStatus::Allowed => {}
        }

        // --- Write-ahead posting marker ---------------------------------
        outbox::set_state(db, &row.dedupe_key, OutboxState::Posting, None)?;

        let event = match row.event() {
            Ok(ev) => ev,
            Err(e) => {
                outbox::set_state(db, &row.dedupe_key, OutboxState::FailedPerma, None)?;
                report
                    .failed
                    .push(("local".to_string(), format!("stored event unparsable: {e}")));
                continue;
            }
        };
        if event.verify().is_err() {
            outbox::set_state(db, &row.dedupe_key, OutboxState::FailedPerma, None)?;
            report
                .failed
                .push(("local".to_string(), "stored event fails signature verification".to_string()));
            continue;
        }
        // Publish-time validation (same policy as save-time).
        if let Err(e) = templates::validate_content(&event.content) {
            outbox::set_state(db, &row.dedupe_key, OutboxState::FailedPerma, None)?;
            report.failed.push((
                "local".to_string(),
                format!("publish-time content validation: {e}"),
            ));
            continue;
        }

        // --- Pre-publish read-back: same content already on a relay? ----
        match publisher.content_already_posted(&event.content).await {
            Ok(true) => {
                handle_delivered(
                    db,
                    &row.dedupe_key,
                    &event.id.to_hex(),
                    &[],
                    Some(&row.per_relay),
                    now_ms,
                )?;
                report.duplicates += 1;
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                // Read-back failing must never block posting — proceed.
                let _ = e;
            }
        }

        // --- Publish & classify -----------------------------------------
        let result: PublishResult = match publisher.publish(&event).await {
            Ok(r) => r,
            Err(e) => {
                // Whole-transport failure (all relays unreachable, publish
                // timeout): transient — the row stays queued for the next
                // flush, and the failure surfaces in the report.
                let mut per_relay = row.per_relay_map();
                per_relay.insert("_transport".to_string(), format!("failed:{e}"));
                outbox::set_state(
                    db,
                    &row.dedupe_key,
                    OutboxState::Queued,
                    Some(&serde_json::to_string(&per_relay).map_err(ShareError::Json)?),
                )?;
                report.failed.push(("_transport".to_string(), e.to_string()));
                report.deferred += 1;
                continue;
            }
        };
        report.failed.extend(result.failures());

        // Merge into the cumulative per-relay map (past relays that went
        // silent stay recorded).
        let mut per_relay = row.per_relay_map();
        for outcome in &result.outcomes {
            per_relay.insert(outcome.relay.clone(), outcome.per_relay_value());
        }
        let per_relay_json = serde_json::to_string(&per_relay).map_err(ShareError::Json)?;

        let any_retryable = result.outcomes.iter().any(|o| o.class.retryable());
        if result.any_delivered() && !any_retryable {
            // Fully converged with delivery somewhere.
            let delivered = result.delivered_relays();
            handle_delivered(
                db,
                &row.dedupe_key,
                &result.event_id,
                &delivered,
                Some(&per_relay_json),
                now_ms,
            )?;
            if result
                .outcomes
                .iter()
                .all(|o| o.class != OkClass::Accepted)
            {
                report.duplicates += 1;
            } else {
                report.posted += 1;
            }
        } else if result.any_delivered() {
            // Delivered somewhere BUT a transient failure remains on another
            // relay: stay queued so the identical bytes are retried; relays
            // that already have it answer `duplicate:` → treated delivered.
            outbox::set_state(db, &row.dedupe_key, OutboxState::Queued, Some(&per_relay_json))?;
            report.deferred += 1;
        } else if any_retryable {
            outbox::set_state(db, &row.dedupe_key, OutboxState::Queued, Some(&per_relay_json))?;
            report.deferred += 1;
        } else {
            // All outcomes terminal and none delivered.
            outbox::set_state(db, &row.dedupe_key, OutboxState::FailedPerma, Some(&per_relay_json))?;
        }
    }
    Ok(report)
}

/// Mark a milestone as delivered: synced LWW marker + outbox `posted` state
/// (optionally refreshing the per-relay map).
fn handle_delivered(
    db: &greek_core::Database,
    dedupe_key: &str,
    event_id: &str,
    delivered_relays: &[String],
    per_relay_json: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    caps::write_marker(db, dedupe_key, event_id, delivered_relays, now_ms)?;
    crate::outbox::set_state(db, dedupe_key, OutboxState::Posted, per_relay_json)?;
    Ok(())
}
