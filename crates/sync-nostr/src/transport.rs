//! Live nostr relay transport: push / fetch_since via NostrSpoolTransport.
//! Does not own the Tokio runtime — all fns are async.

use std::time::Duration;

use nostr_sdk::prelude::*;

use crate::codec::{
    build_checkpoint_event, build_chunk_events, decode_batch_events, CheckpointPayload,
    KIND_BATCH_CHUNK, KIND_CHECKPOINT, TAG_TOPIC,
};
use crate::{Result, SyncBatch, SyncNostrError};

/// Timeout for a single event publish (relay OK ack).
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for REQ -> EOSE collection.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(8);

// SyncBatch / ReviewEvent are re-exported from greek_core::sync (formerly a
// SYNC-CORE-MIRROR local copy; canonicalized at wave 6b).

pub struct NostrSpoolTransport {
    pub client: Client,
    pub keys: Keys,
}

impl NostrSpoolTransport {
    /// Create a transport and connect to the given relays.
    pub async fn connect(keys: Keys, relays: &[&str]) -> Result<Self> {
        let client = Client::new(keys.clone());
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
        for ev in events {
            let out = tokio::time::timeout(SEND_TIMEOUT, self.client.send_event(&ev))
                .await
                .map_err(|_| SyncNostrError::RelayTimeout)?
                .map_err(|e| SyncNostrError::Publish(e.to_string()))?;
            if out.success.is_empty() {
                return Err(SyncNostrError::Publish(format!(
                    "no relay accepted event {}: {:?}",
                    ev.id, out.failed
                )));
            }
        }
        let checkpoint = CheckpointPayload {
            device_id: batch.device_id.clone(),
            last_to_hlc: batch.to_hlc.clone(),
            batch: batch_uuid,
        };
        let cp_event = build_checkpoint_event(&self.keys, &checkpoint)?;
        let out = tokio::time::timeout(SEND_TIMEOUT, self.client.send_event(&cp_event))
            .await
            .map_err(|_| SyncNostrError::RelayTimeout)?
            .map_err(|e| SyncNostrError::Publish(e.to_string()))?;
        if out.success.is_empty() {
            return Err(SyncNostrError::Publish(format!(
                "checkpoint rejected by all relays: {:?}",
                out.failed
            )));
        }
        Ok(())
    }

    /// Fetch all batches with to_hlc > since_hlc, sorted by to_hlc.
    pub async fn fetch_since(&self, since_hlc: Option<&str>) -> Result<Vec<SyncBatch>> {
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

        let batch_events: Vec<&Event> = events
            .iter()
            .filter(|e| e.kind == Kind::Custom(KIND_BATCH_CHUNK))
            .collect();
        let report = decode_batch_events(&self.keys, &batch_events);
        if report.batches.is_empty() && (report.decrypt_failures > 0 || report.partial_batches > 0)
        {
            return Err(SyncNostrError::DecryptFailed {
                count: report.decrypt_failures + report.partial_batches,
            });
        }
        let mut batches = report.batches;
        if let Some(hlc) = since_hlc {
            batches.retain(|b| b.to_hlc.as_str() > hlc);
        }
        batches.sort_by(|a, b| a.to_hlc.cmp(&b.to_hlc));
        Ok(batches)
    }
}
