//! Live nostr relay transport: push / fetch_all via NostrSpoolTransport.
//! Does not own the Tokio runtime — all fns are async.

use std::time::Duration;

use nostr_sdk::prelude::*;

use crate::codec::{
    build_checkpoint_event, build_chunk_events, decode_batch_events_tagged, CheckpointPayload,
    KIND_BATCH_CHUNK, KIND_CHECKPOINT, TAG_TOPIC,
};
use crate::{Result, SyncBatch, SyncNostrError};

/// Timeout for a single event publish (relay OK ack).
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for REQ -> EOSE collection.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(8);

/// Aggregate pull-path bounds (security audit): nostr-sdk caps a single
/// relay wire message at 5 MiB, but nothing bounds how MANY events a
/// misbehaving relay streams into a REQ window before EOSE. A fetch that
/// exceeds either cap fails closed instead of growing memory unboundedly.
pub const MAX_FETCH_EVENTS: usize = 2048;
/// Aggregate cap on chunk/checkpoint content bytes per fetch
/// (≈1000 chunks ≈ 48 MiB plaintext — far above any realistic batch set).
pub const MAX_FETCH_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// Fixed delay inserted between successive event publishes inside `push`
/// (S10). Evidence (SYNC-WIRE.md §2.4): bursting ~140 chunk events from a
/// fresh key trips public-relay rate limiters; 500 ms manual pacing passed
/// live. 200 ms is below that proven-safe cadence but keeps large batches
/// well under burst thresholds (8 chunks ≈ 1.6 s of pacing total).
pub const PUBLISH_PACING: Duration = Duration::from_millis(200);
/// Bounded publish retries per event (S10): at most ONE retry after
/// `PUBLISH_RETRY_BACKOFF` when every relay rejects an event (e.g.
/// `rate-limited: ...`). Retry moves the pacing delay, it does not loop —
/// a permanently-rejecting relay fails the push and preserves the cursor.
pub const PUBLISH_MAX_RETRIES: u32 = 1;
/// Backoff before the single retry of a rejected event (S10): short enough
/// to ride out burst-limit windows, long enough to matter to a limiter
/// keyed on arrival rate.
pub const PUBLISH_RETRY_BACKOFF: Duration = Duration::from_secs(2);

// SyncBatch / ReviewEvent are re-exported from greek_core::sync (formerly a
// SYNC-CORE-MIRROR local copy; canonicalized at wave 6b).

/// One complete batch fetched from the spool, tagged with its wire batch
/// UUID (the chunk "b" tag). S6: pull admission dedups by `batch_uuid` in
/// the `sync_applied_batches` ledger — `batch.to_hlc` no longer filters.
pub struct FetchedBatch {
    pub batch_uuid: String,
    pub batch: SyncBatch,
}

pub struct NostrSpoolTransport {
    pub client: Client,
    pub keys: Keys,
}

impl NostrSpoolTransport {
    /// Create a transport and connect to the given relays.
    pub async fn connect(keys: Keys, relays: &[&str]) -> Result<Self> {
        let client = Client::new(keys.clone());
        // SEMANTIC PIN: sync relies on automatic NIP-42 relay auth (the
        // nostr-sdk default) — relays that AUTH-challenge get signed
        // responses without any action here. Pinned explicitly across the
        // 0.43 → 0.44.1 upgrade; the pool default is still `true` as of
        // nostr-relay-pool 0.44.3.
        client.automatic_authentication(true);
        for r in relays {
            client
                .add_relay(*r)
                .await
                .map_err(|e| SyncNostrError::Connect(format!("{r}: {e}")))?;
        }
        client.connect().await;
        // Wait until at least one relay reports connected (or timeout).
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let relays = client.relays().await;
            if relays.values().any(|r| r.is_connected()) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(SyncNostrError::Connect(
                    "no relay connected within 10s".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(Self { client, keys })
    }

    /// Publish a batch: chunk + encrypt events, require OK ack per event,
    /// then publish the blinded checkpoint event.
    pub async fn push(&self, batch: &SyncBatch) -> Result<()> {
        let events = build_chunk_events(&self.keys, batch)?;
        let batch_uuid = events
            .first()
            .and_then(|e| {
                e.tags.iter().find_map(|t| {
                    let s = t.as_slice();
                    (s.first().map(|x| x.as_str()) == Some("b"))
                        .then(|| s.get(1).cloned())
                        .flatten()
                })
            })
            .unwrap_or_default();
        for (idx, ev) in events.iter().enumerate() {
            // S10 pacing: space out chunk publishes (public relays burst-limit).
            if idx > 0 {
                tokio::time::sleep(PUBLISH_PACING).await;
            }
            self.publish_accepted(ev, "chunk").await?;
        }
        let checkpoint = CheckpointPayload {
            device_id: batch.device_id.clone(),
            last_to_hlc: batch.to_hlc.clone(),
            batch: batch_uuid,
        };
        let cp_event = build_checkpoint_event(&self.keys, &checkpoint)?;
        // Same pacing/retry policy covers the checkpoint publish.
        tokio::time::sleep(PUBLISH_PACING).await;
        self.publish_accepted(&cp_event, "checkpoint").await?;
        Ok(())
    }

    /// Publish one event requiring at least one relay OK ack, with S10's
    /// bounded retry policy: on an all-relay rejection (`success` empty —
    /// the rejection bucket in this codebase's error taxonomy) or a pool
    /// error, back off `PUBLISH_RETRY_BACKOFF` once and retry at most
    /// `PUBLISH_MAX_RETRIES` times. Timeouts are not retried: the event may
    /// already be admitted relay-side, and the taxonomic distinction keeps
    /// ambiguous outcomes fail-fast. Never loops past the bound.
    async fn publish_accepted(&self, ev: &Event, what: &str) -> Result<()> {
        let mut attempt = 0u32;
        loop {
            let out = match tokio::time::timeout(SEND_TIMEOUT, self.client.send_event(ev)).await {
                Err(_) => return Err(SyncNostrError::RelayTimeout),
                Ok(res) => res,
            };
            let rejection = match out {
                Ok(out) if !out.success.is_empty() => return Ok(()),
                Ok(out) => format!(
                    "{what} event {} rejected by all relays: {:?}",
                    ev.id, out.failed
                ),
                Err(e) => format!("{what} event {} publish error: {e}", ev.id),
            };
            if attempt >= PUBLISH_MAX_RETRIES {
                return Err(SyncNostrError::Publish(rejection));
            }
            attempt += 1;
            tokio::time::sleep(PUBLISH_RETRY_BACKOFF).await;
        }
    }

    /// Fetch ALL complete batches on the spool, each tagged with its wire
    /// batch UUID (the chunk "b" tag), sorted by `to_hlc`.
    ///
    /// S6: no HLC filter here. Admission is the CALLER's job via the
    /// `sync_applied_batches` UUID ledger — a batch admitted at ANY HLC
    /// position — so a batch that was incomplete (or pushed from a lagging
    /// clock) when the pull high-water mark advanced past its HLCs still
    /// surfaces once complete. Incomplete batches are tolerated (counted,
    /// not returned) so a later fetch can complete them; complete-but-
    /// corrupt batches are logged, skipped and never surfaced for marking.
    pub async fn fetch_all(&self) -> Result<Vec<FetchedBatch>> {
        let filter = Filter::new()
            .author(self.keys.public_key())
            .kinds([
                Kind::Custom(KIND_BATCH_CHUNK),
                Kind::Custom(KIND_CHECKPOINT),
            ])
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::T),
                TAG_TOPIC,
            );
        let events = tokio::time::timeout(
            FETCH_TIMEOUT,
            self.client.fetch_events(filter, FETCH_TIMEOUT),
        )
        .await
        .map_err(|_| SyncNostrError::RelayTimeout)?
        .map_err(|e| SyncNostrError::Nostr(e.to_string()))?;

        // Aggregate fetch bounds (see MAX_FETCH_EVENTS / MAX_FETCH_TOTAL_BYTES):
        // fail closed when a relay floods the REQ window.
        if events.len() > MAX_FETCH_EVENTS {
            return Err(SyncNostrError::Nostr(format!(
                "relay returned {} events (cap {MAX_FETCH_EVENTS}); refusing to process",
                events.len()
            )));
        }
        let total_bytes: usize = events.iter().map(|e| e.content.len()).sum();
        if total_bytes > MAX_FETCH_TOTAL_BYTES {
            return Err(SyncNostrError::Nostr(format!(
                "relay returned {total_bytes} content bytes (cap {MAX_FETCH_TOTAL_BYTES}); refusing to process"
            )));
        }

        let batch_events: Vec<&Event> = events
            .iter()
            .filter(|e| e.kind == Kind::Custom(KIND_BATCH_CHUNK))
            .collect();
        let report = decode_batch_events_tagged(&self.keys, &batch_events);
        if report.batches.is_empty() && (report.decrypt_failures > 0 || report.partial_batches > 0)
        {
            return Err(SyncNostrError::DecryptFailed {
                count: report.decrypt_failures + report.partial_batches,
            });
        }
        if report.decrypt_failures > 0 || report.partial_batches > 0 {
            // S6 observability: dropped batches stay UNMARKED — incomplete
            // ones complete on a later fetch; corrupt ones are skipped
            // here, never crash, never recorded as applied.
            eprintln!(
                "sync-nostr fetch: skipped {} corrupt chunk group(s), {} incomplete batch(es)",
                report.decrypt_failures, report.partial_batches
            );
        }
        let mut batches: Vec<FetchedBatch> = report
            .batches
            .into_iter()
            .map(|t| FetchedBatch {
                batch_uuid: t.batch_uuid,
                batch: t.batch,
            })
            .collect();
        batches.sort_by(|a, b| a.batch.to_hlc.cmp(&b.batch.to_hlc));
        Ok(batches)
    }
}
