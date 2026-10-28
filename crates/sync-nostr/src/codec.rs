//! Pure in-process codec: SyncBatch <-> encrypted nostr event payloads.
//! No network access in this module (unit-testable).

use hmac::{Hmac, Mac};
use nostr_sdk::nips::nip44;
use nostr_sdk::prelude::*;
use sha2::Sha256;
use uuid::Uuid;

use crate::{Result, ReviewEvent, SyncBatch, SyncNostrError};

/// Maximum plaintext chunk size (bytes) per kind-1078 event.
///
/// S10: 32 KiB plaintext so the NIP-44-v2 base64 ciphertext stays well under
/// the 64 KiB relay content limit BEFORE event metadata. Size math (v2):
///   raw = 1 (version) + 32 (nonce) + 2 (len prefix) + padded(32768) + 32 (MAC)
///   padded(32768) = 32768   (padding granularity 4096: exact multiple, +0)
///   raw = 32835 B  ->  base64 = 4 * ceil(32835/3) = 43780 B (~42.75 KiB)
/// Plus the event JSON envelope (id/pubkey/sig/tags, ~1 KiB) this is far
/// below 65536 B; the old 48 KiB chunk produced 65628 B of content alone.
pub const CHUNK_SIZE: usize = 32 * 1024;

/// NIP-01 "regular" (relay-retained) kind for sync batch chunks.
pub const KIND_BATCH_CHUNK: u16 = 1078;
/// NIP-78 parameterized-replaceable kind for the checkpoint event.
pub const KIND_CHECKPOINT: u16 = 30078;

pub const TAG_TOPIC: &str = "greek-sync";

/// Serialize a batch, then chunk the plaintext at CHUNK_SIZE.
/// Splits land at most CHUNK_SIZE bytes apart and ALWAYS on UTF-8 char
/// boundaries (we walk back from the hard cap if it falls inside a multibyte
/// codepoint, ≤3 bytes for valid UTF-8). Decodes bytes end-to-end regardless
/// (see `nip44_decrypt_self`); boundary-safe splits are cheap insurance for
/// any consumer that still round-trips a single chunk through `str`.
pub fn chunk_batch(batch: &SyncBatch) -> Result<Vec<Vec<u8>>> {
    let json = serde_json::to_vec(batch)?;
    Ok(split_utf8_safe(&json, CHUNK_SIZE))
}

/// Split `buf` into chunks of at most `max` bytes, never bisecting a UTF-8
/// codepoint. `buf` must be valid UTF-8 (serde_json output is).
fn split_utf8_safe(buf: &[u8], max: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = buf;
    while rest.len() > max {
        let mut end = max;
        // A UTF-8 char boundary is an index whose byte is NOT a continuation
        // byte (0b10xxxxxx); walking back from the cap reaches one in <=3
        // steps for valid UTF-8 (serde_json output).
        while rest[end] & 0xC0 == 0x80 {
            end -= 1;
        }
        assert!(end > 0, "valid UTF-8 has a boundary within 3 bytes of the cap");
        out.push(rest[..end].to_vec());
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        out.push(rest.to_vec());
    }
    out
}

/// Build the kind-1078 events for one batch (NIP-44 encrypted to self per chunk).
/// Each event: kind 1078, tags ["t","greek-sync"], ["d",device_id],
/// ["b",batch_uuid], ["i",idx], ["n",count].
pub fn build_chunk_events(keys: &Keys, batch: &SyncBatch) -> Result<Vec<Event>> {
    let chunks = chunk_batch(batch)?;
    build_events_from_chunks(keys, &batch.device_id, chunks)
}

/// Inner event builder over explicit plaintext chunks; factored out so tests
/// can drive deliberately byte-arbitrary (mid-codepoint) splits (S10 probe).
fn build_events_from_chunks(
    keys: &Keys,
    device_id: &str,
    chunks: Vec<Vec<u8>>,
) -> Result<Vec<Event>> {
    let count = chunks.len();
    let batch_uuid = Uuid::new_v4().to_string();
    let mut events = Vec::with_capacity(count);
    for (idx, chunk) in chunks.iter().enumerate() {
        let ct = nip44_encrypt_self(keys, chunk)?;
        let builder = EventBuilder::new(Kind::Custom(KIND_BATCH_CHUNK), ct).tags([
            Tag::parse(["t", TAG_TOPIC]).map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
            Tag::parse(["d", device_id]).map_err(|e| SyncNostrError::Nostr(e.to_string()))?,
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

/// NIP-44 decrypt from self — to BYTES.
///
/// S10: `nip44::decrypt_to_bytes` skips the UTF-8 roundtrip of the old
/// `decrypt` -> `String` path. Per-chunk UTF-8 validation was the killer: a
/// chunk split inside a multibyte Greek char failed `String::from_utf8`,
/// zeroing the whole batch. Chunks are now decrypted as opaque bytes and the
/// reassembled payload is validated ONCE as a whole (`reassemble_chunks`).
pub fn nip44_decrypt_self(keys: &Keys, ciphertext: &str) -> Result<Vec<u8>> {
    let pk = keys.public_key();
    nip44::decrypt_to_bytes(keys.secret_key(), &pk, ciphertext)
        .map_err(|_e| SyncNostrError::DecryptFailed { count: 1 })
}

/// Reassemble ordered chunks into a SyncBatch.
///
/// S10: chunks are raw BYTES (see `nip44_decrypt_self`); concatenation is
/// pure byte-append and UTF-8 is validated exactly ONCE, here, over the whole
/// payload (`serde_json::from_slice` validates as it parses — there is no
/// per-chunk string roundtrip anywhere in the decode path).
///
/// Version gate (SYNC-WIRE.md §2.1): the parsed batch must pass the SHARED
/// `greek_core::sync::validate_batch` policy — v1 and v2 are accepted,
/// anything newer (or 0) is rejected HERE at the decode boundary rather
/// than flowing half-applied into the merge layer. A rejection counts as a
/// failed batch in `decode_batch_events` (dropped and counted), consistent
/// with corrupt-cipher handling.
pub fn reassemble_chunks(chunks: &[Vec<u8>]) -> Result<SyncBatch> {
    let mut buf = Vec::new();
    for c in chunks {
        buf.extend_from_slice(c);
    }
    let batch: SyncBatch = serde_json::from_slice(&buf)?;
    if let Err(_) = greek_core::sync::validate_batch(&batch) {
        return Err(SyncNostrError::UnsupportedVersion(batch.v));
    }
    Ok(batch)
}

/// Given all kind-1078 events for our pubkey (any sender-side validity already
/// checked by nostr-sdk), regroup by "b" tag, order by "i", decrypt, and
/// return complete, parseable batches.
///
/// Events that fail decrypt/parse are dropped and counted; batches missing
/// chunks are reported via SyncNostrError::PartialBatch only if the caller
/// asks for strictness — here incomplete batches are simply skipped and
/// counted in `decrypt_failures`/`partial` for observability.
/// A complete, parseable batch tagged with its wire batch UUID (the chunk
/// "b" tag) — S6 pull admission dedups per UUID.
pub struct TaggedBatch {
    pub batch_uuid: String,
    pub batch: SyncBatch,
}

pub struct DecodeReport {
    pub batches: Vec<SyncBatch>,
    pub decrypt_failures: usize,
    pub partial_batches: usize,
}

/// Same as [`decode_batch_events`] but each returned batch keeps its "b"
/// tag UUID, so the pull path can admit batches at ANY HLC position and
/// dedup by identity instead of by a global cursor high-water mark.
pub struct TaggedDecodeReport {
    pub batches: Vec<TaggedBatch>,
    pub decrypt_failures: usize,
    pub partial_batches: usize,
}

pub fn decode_batch_events_tagged(keys: &Keys, events: &[&Event]) -> TaggedDecodeReport {
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

    let mut report = TaggedDecodeReport {
        batches: Vec::new(),
        decrypt_failures: 0,
        partial_batches: 0,
    };

    for (b, mut chunks) in groups {
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
            Ok(batch) => report.batches.push(TaggedBatch {
                batch_uuid: b,
                batch,
            }),
            Err(_) => report.decrypt_failures += 1,
        }
    }
    report
}

pub fn decode_batch_events(keys: &Keys, events: &[&Event]) -> DecodeReport {
    let tagged = decode_batch_events_tagged(keys, events);
    DecodeReport {
        batches: tagged.batches.into_iter().map(|t| t.batch).collect(),
        decrypt_failures: tagged.decrypt_failures,
        partial_batches: tagged.partial_batches,
    }
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
        known_words: Vec::new(),
        lookup_cards: Vec::new(),
        chapter_unlocks: Vec::new(),
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

    /// Wire-version gate at the reassembly boundary: v1 and v2 payloads
    /// decode; an unknown FUTURE version is rejected here — identically to
    /// the file-import gate (`greek_core::sync::validate_batch`) — instead
    /// of reaching the merge layer.
    #[test]
    fn reassemble_rejects_unsupported_wire_version() {
        for v in [1, 2] {
            let mut batch = synthetic_batch("dev-v", 1);
            batch.v = v;
            let chunks = chunk_batch(&batch).unwrap();
            let back = reassemble_chunks(&chunks).unwrap();
            assert_eq!(back.v, v);
        }
        // A v1-era JSON document (no v2 collection keys) also decodes,
        // with empty content collections (serde defaults).
        let v1_json = r#"{"v":1,"device_id":"dev","from_hlc":"","to_hlc":"1762000000000-0000-feedface","reviews":[],"settings":[]}"#;
        let batch = reassemble_chunks(&[v1_json.as_bytes().to_vec()]).unwrap();
        assert_eq!(batch.v, 1);
        assert!(batch.known_words.is_empty()
            && batch.lookup_cards.is_empty()
            && batch.chapter_unlocks.is_empty());

        let mut future = synthetic_batch("dev-f", 1);
        future.v = 99;
        let chunks = chunk_batch(&future).unwrap();
        match reassemble_chunks(&chunks) {
            Err(SyncNostrError::UnsupportedVersion(99)) => {}
            other => panic!("v99 must be UnsupportedVersion(99), got {other:?}"),
        }
    }

    #[test]
    fn max_chunk_size_boundary() {
        // Find a review count producing > 200 KiB of JSON.
        let batch = synthetic_batch("dev-big", 1500);
        let json_len = serde_json::to_vec(&batch).unwrap().len();
        assert!(json_len > 200 * 1024, "need >200KiB, got {}", json_len);
        let chunks = chunk_batch(&batch).unwrap();
        assert!(chunks.len() >= 4);
        for (i, c) in chunks.iter().enumerate() {
            assert!(c.len() <= CHUNK_SIZE);
            if i + 1 < chunks.len() {
                // ASCII-only payload: every byte offset is a char boundary,
                // so boundary-safe splits still fill to the exact cap.
                assert_eq!(c.len(), CHUNK_SIZE);
            }
        }
        let back = reassemble_chunks(&chunks).unwrap();
        assert_eq!(batch, back);
    }

    /// S10 belt-and-braces: chunk_batch never bisects a UTF-8 codepoint, so
    /// every chunk is independently a valid `str` (Greek-heavy payload).
    #[test]
    fn chunk_splits_stay_on_utf8_boundaries() {
        let mut batch = synthetic_batch("dev-greek-boundary", 1);
        // ~70 KiB run of a 2-byte Greek char: several splits must walk back.
        batch.reviews[0].card_id = "α".repeat(35_000);
        let json = serde_json::to_vec(&batch).unwrap();
        let chunks = chunk_batch(&batch).unwrap();
        assert!(chunks.len() >= 3, "expected multi-chunk, got {}", chunks.len());
        let mut concat = Vec::new();
        for (i, c) in chunks.iter().enumerate() {
            assert!(c.len() <= CHUNK_SIZE);
            std::str::from_utf8(c).expect("each chunk must be valid UTF-8 on its own");
            if i + 1 < chunks.len() {
                assert!(
                    c.len() >= CHUNK_SIZE - 3,
                    "walk-back is at most 3 bytes, got {}",
                    c.len()
                );
            }
            concat.extend_from_slice(c);
        }
        assert_eq!(concat, json, "concatenated chunks must equal the JSON");
        assert_eq!(reassemble_chunks(&chunks).unwrap(), batch);
    }

    /// S10 AUDIT PROBE (regression): a multichunk (>=3 chunks) Greek batch
    /// split at DELIBERATELY byte-arbitrary offsets that land mid-codepoint
    /// (the pre-fix chunking), taken through the full encrypted
    /// self-roundtrip. Pre-fix, the String-returning decrypt path failed
    /// UTF-8 on the bisected chunks -> ZERO recovered batches. Post-fix the
    /// decode path is bytes end-to-end and the batch reassembles exactly.
    #[test]
    fn greek_mid_codepoint_split_encrypted_roundtrip() {
        let keys = Keys::generate();
        let mut batch = synthetic_batch("dev-greek-probe", 1);
        // Find an ASCII pad so the raw 32 KiB split offsets fall INSIDE a
        // 2-byte Greek char: pad shifts the α-run start by 1-byte steps and
        // both raw boundaries are an even distance apart, so one pad parity
        // fixes both. Assert a solution exists (construction guarantee).
        let mut json = Vec::new();
        let mut boundary_offsets = Vec::new();
        let mut found = false;
        for pad in 0..8usize {
            batch.reviews[0].card_id = format!("{}{}", "A".repeat(pad), "α".repeat(40_000));
            json = serde_json::to_vec(&batch).unwrap();
            assert!(
                json.len() > 2 * CHUNK_SIZE,
                "probe payload must be >=3 chunks, got {}",
                json.len()
            );
            let mid: Vec<usize> = (1..=(json.len() - 1) / CHUNK_SIZE)
                .map(|k| k * CHUNK_SIZE)
                // continuation byte 0b10xxxxxx == split inside a codepoint
                .filter(|&b| json[b] & 0xC0 == 0x80)
                .collect();
            if mid.len() == (json.len() - 1) / CHUNK_SIZE {
                boundary_offsets = mid;
                found = true;
                break;
            }
        }
        assert!(
            found && boundary_offsets.len() >= 2,
            "pad search must land every raw boundary mid-codepoint"
        );

        // Split at the RAW byte offsets (pre-fix behavior) and encrypt per
        // chunk, exactly as the production event builder does.
        let raw_chunks: Vec<Vec<u8>> = json.chunks(CHUNK_SIZE).map(|c| c.to_vec()).collect();
        assert!(raw_chunks.len() >= 3);
        assert!(
            raw_chunks.iter().any(|c| std::str::from_utf8(c).is_err()),
            "at least one raw chunk is invalid UTF-8 alone (probe precondition)"
        );
        let events = build_events_from_chunks(&keys, &batch.device_id, raw_chunks.clone())
            .unwrap();

        // Every chunk decrypts to its exact bytes...
        for (i, ev) in events.iter().enumerate() {
            let plain = nip44_decrypt_self(&keys, &ev.content)
                .unwrap_or_else(|e| panic!("chunk {i} must decrypt to bytes: {e}"));
            assert_eq!(plain, raw_chunks[i], "chunk {i} bytes must roundtrip");
        }
        // ...and the full decode path reassembles the batch byte-identical.
        let refs: Vec<&Event> = events.iter().collect();
        let report = decode_batch_events(&keys, &refs);
        assert_eq!(report.decrypt_failures, 0, "pre-fix this was >0");
        assert_eq!(report.partial_batches, 0);
        assert_eq!(report.batches, vec![batch]);
    }

    /// S10 coupled P2: a MAX-SIZE plaintext chunk must encrypt to an event
    /// content that stays comfortably under the 64 KiB relay limit. Math
    /// pinned here (see CHUNK_SIZE doc): NIP-44 v2 raw = 1 (version) +
    /// 32 (nonce) + 2 (len prefix) + 32768 (padded plaintext, granularity
    /// 4096 == exact multiple, +0) + 32 (MAC) = 32835 B; base64 inflates to
    /// exactly 4 * (32835/3) = 43780 B (~42.75 KiB).
    #[test]
    fn max_chunk_ciphertext_within_48_kib() {
        let keys = Keys::generate();
        let chunk = vec![b'x'; CHUNK_SIZE];
        let ct = nip44_encrypt_self(&keys, &chunk).unwrap();
        assert_eq!(
            ct.len(),
            43780,
            "documented NIP-44 v2 size math drifted (raw 32835 -> base64 43780)"
        );
        assert!(
            ct.len() <= 48 * 1024,
            "max chunk ciphertext {} B must stay <= 48 KiB",
            ct.len()
        );
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

    /// S6: the tagged decode surfaces each complete batch's "b"-tag UUID so
    /// pull admission can dedup per batch — a batch incomplete on one fetch
    /// (missing final chunk) completes and surfaces on the next.
    #[test]
    fn tagged_decode_surfaces_uuid_and_late_chunk_completion() {
        let keys = Keys::generate();
        let batch = synthetic_batch("dev-s6-tagged", 800); // multi-chunk
        let events = build_chunk_events(&keys, &batch).unwrap();
        assert!(events.len() >= 2, "test payload must be multi-chunk");
        let batch_uuid = {
            let tags = events[0].tags.clone();
            tags.iter()
                .find_map(|t| {
                    let s = t.as_slice();
                    (s.first().map(|x| x.as_str()) == Some("b"))
                        .then(|| s[1].to_string())
                })
                .expect("b tag present")
        };

        // Missing final chunk: complete batches NONE, partial counted —
        // nothing surfaces, nothing can be marked as applied.
        let partial_refs: Vec<&Event> = events[..events.len() - 1].iter().collect();
        let report = decode_batch_events_tagged(&keys, &partial_refs);
        assert_eq!(report.partial_batches, 1);
        assert!(report.batches.is_empty());

        // Late final chunk arrives: the now-complete batch surfaces WITH
        // its wire UUID (regardless of any HLC cursor position).
        let full_refs: Vec<&Event> = events.iter().collect();
        let report = decode_batch_events_tagged(&keys, &full_refs);
        assert_eq!(report.partial_batches, 0);
        assert_eq!(report.decrypt_failures, 0);
        assert_eq!(report.batches.len(), 1);
        assert_eq!(report.batches[0].batch_uuid, batch_uuid);
        assert_eq!(report.batches[0].batch, batch);

        // Complete-but-corrupt (blob that fails the wire-version gate at
        // reassembly): dropped and counted, never surfaced for marking.
        let corrupt = r#"{"v":99,"device_id":"d","from_hlc":"","to_hlc":"","reviews":[],"settings":[]}"#;
        let ct = nip44_encrypt_self(&keys, corrupt.as_bytes()).unwrap();
        let bad_ev = EventBuilder::new(Kind::Custom(KIND_BATCH_CHUNK), ct)
            .tags([
                Tag::parse(["t", TAG_TOPIC]).unwrap(),
                Tag::parse(["b", "corrupt-uuid"]).unwrap(),
                Tag::parse(["i", "0"]).unwrap(),
                Tag::parse(["n", "1"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        let mut refs: Vec<&Event> = events.iter().collect();
        refs.push(&bad_ev);
        let report = decode_batch_events_tagged(&keys, &refs);
        assert_eq!(report.batches.len(), 1, "good batch still surfaces");
        assert!(
            report.batches.iter().all(|t| t.batch_uuid != "corrupt-uuid"),
            "corrupt batch must never surface"
        );
        assert_eq!(report.decrypt_failures, 1);
    }

    #[test]
    fn checkpoint_d_tag_deterministic_and_blind() {let keys = Keys::generate();
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
