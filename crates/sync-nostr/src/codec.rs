//! Pure in-process codec: SyncBatch <-> encrypted nostr event payloads.
//! No network access in this module (unit-testable).

use hmac::{Hmac, Mac};
use nostr_sdk::nips::nip44;
use nostr_sdk::prelude::*;
use sha2::Sha256;
use uuid::Uuid;

use crate::{Result, ReviewEvent, SyncBatch, SyncNostrError};

/// Maximum plaintext chunk size (bytes) per kind-1078 event.
pub const CHUNK_SIZE: usize = 48 * 1024;

/// NIP-01 "regular" (relay-retained) kind for sync batch chunks.
pub const KIND_BATCH_CHUNK: u16 = 1078;
/// NIP-78 parameterized-replaceable kind for the checkpoint event.
pub const KIND_CHECKPOINT: u16 = 30078;

pub const TAG_TOPIC: &str = "greek-sync";

/// Serialize a batch, then chunk the plaintext at CHUNK_SIZE.
/// Returns UTF-8-safe byte chunks (JSON slices on byte boundaries are fine
/// because we reassemble bytes, not strings).
pub fn chunk_batch(batch: &SyncBatch) -> Result<Vec<Vec<u8>>> {
    let json = serde_json::to_vec(batch)?;
    Ok(json
        .chunks(CHUNK_SIZE)
        .map(|c| c.to_vec())
        .collect::<Vec<_>>())
}

/// Build the kind-1078 events for one batch (NIP-44 encrypted to self per chunk).
/// Each event: kind 1078, tags ["t","greek-sync"], ["d",device_id],
/// ["b",batch_uuid], ["i",idx], ["n",count].
pub fn build_chunk_events(keys: &Keys, batch: &SyncBatch) -> Result<Vec<Event>> {
    let chunks = chunk_batch(batch)?;
    let count = chunks.len();
    let batch_uuid = Uuid::new_v4().to_string();
    let mut events = Vec::with_capacity(count);
    for (idx, chunk) in chunks.iter().enumerate() {
        let ct = nip44_encrypt_self(keys, chunk)?;
        let builder = EventBuilder::new(Kind::Custom(KIND_BATCH_CHUNK), ct).tags([
            Tag::parse(["t", TAG_TOPIC]).map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
            Tag::parse(["d", &batch.device_id]).map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
            Tag::parse(["b", &batch_uuid]).map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
            Tag::parse(["i", &idx.to_string()]).map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
            Tag::parse(["n", &count.to_string()])
                .map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
        ]);
        events.push(
            builder
                .sign_with_keys(keys)
                .map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
        );
    }
    Ok(events)
}

/// NIP-44 v2 encrypt to self (conversation key with own pubkey).
pub fn nip44_encrypt_self(keys: &Keys, plaintext: &[u8]) -> Result<String> {
    let pk = keys.public_key();
    nip44::encrypt(
        keys.secret_key(),
        &pk,
        plaintext,
        nip44::Version::V2,
    )
    .map_err(|e| SyncNostrError::Nostr(e.to_string()))
}

/// NIP-44 decrypt from self.
pub fn nip44_decrypt_self(keys: &Keys, ciphertext: &str) -> Result<Vec<u8>> {
    let pk = keys.public_key();
    nip44::decrypt(keys.secret_key(), &pk, ciphertext)
        .map_err(|_e| SyncNostrError::DecryptFailed { count: 1 })
        .map(|s| s.into_bytes())
}

/// Reassemble ordered chunks into a SyncBatch.
pub fn reassemble_chunks(chunks: &[Vec<u8>]) -> Result<SyncBatch> {
    let mut buf = Vec::new();
    for c in chunks {
        buf.extend_from_slice(c);
    }
    Ok(serde_json::from_slice(&buf)?)
}

/// Given all kind-1078 events for our pubkey (any sender-side validity already
/// checked by nostr-sdk), regroup by "b" tag, order by "i", decrypt, and
/// return complete, parseable batches.
///
/// Events that fail decrypt/parse are dropped and counted; batches missing
/// chunks are reported via SyncNostrError::PartialBatch only if the caller
/// asks for strictness — here incomplete batches are simply skipped and
/// counted in `decrypt_failures`/`partial` for observability.
pub struct DecodeReport {
    pub batches: Vec<SyncBatch>,
    pub decrypt_failures: usize,
    pub partial_batches: usize,
}

pub fn decode_batch_events(keys: &Keys, events: &[&Event]) -> DecodeReport {
    use std::collections::HashMap;
    let mut groups: HashMap<String, Vec<(u64, u64, &Event)>> = HashMap::new();
    for ev in events {
        let mut b = None;
        let mut i = None;
        let mut n = None;
        for tag in ev.tags.iter() {
            let slice = tag.as_slice();
            match slice.first().map(|s| s.as_str()) {
                Some("b") => b = slice.get(1).map(|s| s.to_string()),
                Some("i") => i = slice.get(1).and_then(|s| s.parse::<u64>().ok()),
                Some("n") => n = slice.get(1).and_then(|s| s.parse::<u64>().ok()),
                _ => {}
            }
        }
        if let (Some(b), Some(i), Some(n)) = (b, i, n) {
            groups.entry(b).or_default().push((i, n, ev));
        }
    }

    let mut report = DecodeReport {
        batches: Vec::new(),
        decrypt_failures: 0,
        partial_batches: 0,
    };

    for (_b, mut chunks) in groups {
        chunks.sort_by_key(|(i, _, _)| *i);
        let expected_n = chunks.first().map(|(_, n, _)| *n).unwrap_or(0);
        let have: std::collections::HashSet<u64> = chunks.iter().map(|(i, _, _)| *i).collect();
        if expected_n == 0 || (0..expected_n).any(|k| !have.contains(&k)) {
            report.partial_batches += 1;
            continue;
        }
        let mut plain: Vec<Vec<u8>> = Vec::with_capacity(chunks.len());
        let mut failed = false;
        for (_, _, ev) in &chunks {
            match nip44_decrypt_self(keys, &ev.content) {
                Ok(p) => plain.push(p),
                Err(_) => {
                    report.decrypt_failures += 1;
                    failed = true;
                }
            }
        }
        if failed {
            continue;
        }
        match reassemble_chunks(&plain) {
            Ok(batch) => report.batches.push(batch),
            Err(_) => report.decrypt_failures += 1,
        }
    }
    report
}

/// Blinded checkpoint d-tag:
/// hex HMAC-SHA256(nip44_conversation_key, "greek-sync/checkpoint").
pub fn checkpoint_d_tag(keys: &Keys) -> Result<String> {
    let ck = nip44::v2::ConversationKey::derive(keys.secret_key(), &keys.public_key())
        .map_err(|e| SyncNostrError::Nostr(e.to_string()))?;
    let mut mac = Hmac::<Sha256>::new_from_slice(ck.as_bytes())
        .map_err(|e| SyncNostrError::Nostr(e.to_string()))?;
    mac.update(b"greek-sync/checkpoint");
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// Checkpoint payload JSON (plaintext before NIP-44 self-encryption).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointPayload {
    pub device_id: String,
    pub last_to_hlc: String,
    pub batch: String,
}

/// Build the kind-30078 checkpoint event (content NIP-44-encrypted to self).
pub fn build_checkpoint_event(keys: &Keys, payload: &CheckpointPayload) -> Result<Event> {
    let dtag = checkpoint_d_tag(keys)?;
    let ct = nip44_encrypt_self(keys, serde_json::to_string(payload)?.as_bytes())?;
    EventBuilder::new(Kind::Custom(KIND_CHECKPOINT), ct)
        .tags([
            Tag::parse(["t", TAG_TOPIC]).map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
            Tag::parse(["d", &dtag]).map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
        ])
        .sign_with_keys(keys)
        .map_err(|e| SyncNostrError::Nostr(e.to_string()))
}

/// Decode a checkpoint event's content.
pub fn decode_checkpoint_event(keys: &Keys, event: &Event) -> Result<CheckpointPayload> {
    let plain = nip44_decrypt_self(keys, &event.content)?;
    Ok(serde_json::from_slice(&plain)?)
}

/// Helper for tests: fabricate a batch with n synthetic reviews.
pub fn synthetic_batch(device_id: &str, n: usize) -> SyncBatch {
    let reviews = (0..n)
        .map(|i| ReviewEvent {
            uuid: Uuid::new_v4().to_string(),
            card_id: format!("card-{:06}", i),
            timestamp: format!("2026-10-17T12:{:02}:{:02}Z", i % 60, i % 60),
            rating: (i % 4 + 1) as u32,
            elapsed_days: (i as f64) * 1.5,
            scheduled_days: (i as f64) * 2.5,
            source: "review".to_string(),
            device_id: device_id.to_string(),
            hlc: format!("2026-10-17T12:00:00Z-{:04}-dev", i),
        })
        .collect();
    SyncBatch {
        v: 1,
        device_id: device_id.to_string(),
        from_hlc: "2026-10-17T00:00:00Z-0000-dev".to_string(),
        to_hlc: "2026-10-17T13:00:00Z-0000-dev".to_string(),
        reviews,
        settings: vec![(
            "new_per_day".to_string(),
            "20".to_string(),
            "2026-10-17T12:00:00Z-0000-dev".to_string(),
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_and_reassemble_roundtrip() {
        let batch = synthetic_batch("dev-1", 50);
        let chunks = chunk_batch(&batch).unwrap();
        assert_eq!(chunks.len(), 1, "small batch must fit in one chunk");
        let back = reassemble_chunks(&chunks).unwrap();
        assert_eq!(batch, back);
    }

    #[test]
    fn forty_eight_kib_boundary() {
        // Find a review count producing > 200 KiB of JSON.
        let batch = synthetic_batch("dev-big", 1500);
        let json_len = serde_json::to_vec(&batch).unwrap().len();
        assert!(json_len > 200 * 1024, "need >200KiB, got {}", json_len);
        let chunks = chunk_batch(&batch).unwrap();
        assert!(chunks.len() >= 4);
        for (i, c) in chunks.iter().enumerate() {
            assert!(c.len() <= CHUNK_SIZE);
            if i + 1 < chunks.len() {
                assert_eq!(c.len(), CHUNK_SIZE);
            }
        }
        let back = reassemble_chunks(&chunks).unwrap();
        assert_eq!(batch, back);
    }

    #[test]
    fn full_event_roundtrip_encrypted() {
        let keys = Keys::generate();
        let batch = synthetic_batch("dev-2", 800);
        let events = build_chunk_events(&keys, &batch).unwrap();
        assert!(events.len() >= 2, "expected multi-chunk");
        for ev in &events {
            assert_eq!(ev.kind, Kind::Custom(KIND_BATCH_CHUNK));
            assert!(ev.verify().is_ok(), "signature must verify");
            assert!(ev.tags.iter().any(|t| t.as_slice()[0] == "t"
                && t.as_slice().get(1).map(|s| s.as_str()) == Some(TAG_TOPIC)));
        }
        let refs: Vec<&Event> = events.iter().collect();
        let report = decode_batch_events(&keys, &refs);
        assert_eq!(report.decrypt_failures, 0);
        assert_eq!(report.partial_batches, 0);
        assert_eq!(report.batches, vec![batch]);
    }

    #[test]
    fn wrong_key_drops_and_counts() {
        let alice = Keys::generate();
        let mallory = Keys::generate();
        let batch = synthetic_batch("dev-3", 10);
        let events = build_chunk_events(&alice, &batch).unwrap();
        let refs: Vec<&Event> = events.iter().collect();
        let report = decode_batch_events(&mallory, &refs);
        assert_eq!(report.batches.len(), 0);
        assert!(report.decrypt_failures > 0);
    }

    #[test]
    fn checkpoint_d_tag_deterministic_and_blind() {
        let keys = Keys::generate();
        let a = checkpoint_d_tag(&keys).unwrap();
        let b = checkpoint_d_tag(&keys).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // hex of 32 bytes
        assert!(!a.contains("checkpoint")); // blinded: no semantic label
        let other = Keys::generate();
        assert_ne!(a, checkpoint_d_tag(&other).unwrap());
    }

    #[test]
    fn checkpoint_event_roundtrip_missing_chunk_detected() {
        let keys = Keys::generate();
        let payload = CheckpointPayload {
            device_id: "dev-1".into(),
            last_to_hlc: "2026-10-17T13:00:00Z-0000-dev".into(),
            batch: Uuid::new_v4().to_string(),
        };
        let ev = build_checkpoint_event(&keys, &payload).unwrap();
        assert_eq!(ev.kind, Kind::Custom(KIND_CHECKPOINT));
        assert!(ev.verify().is_ok());
        let back = decode_checkpoint_event(&keys, &ev).unwrap();
        assert_eq!(back.device_id, payload.device_id);
        assert_eq!(back.last_to_hlc, payload.last_to_hlc);
        assert_eq!(back.batch, payload.batch);

        // partial batch detection
        let batch = synthetic_batch("dev-4", 800);
        let mut events = build_chunk_events(&keys, &batch).unwrap();
        assert!(events.len() >= 2);
        events.pop(); // drop last chunk
        let refs: Vec<&Event> = events.iter().collect();
        let report = decode_batch_events(&keys, &refs);
        assert_eq!(report.partial_batches, 1);
        assert_eq!(report.batches.len(), 0);
    }
}
