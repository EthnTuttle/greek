//! S6 regression tests: pull admission is per-batch UUID, not the HLC
//! cursor.
//!
//! The S6 bug (production audit): `fetch_since` filtered every decoded
//! batch at/below the single global `nostr_last_pull_hlc` cursor and
//! `sync_nostr_pull` advanced that cursor to the max returned `to_hlc`.
//! A batch that was INCOMPLETE when a newer complete batch advanced the
//! cursor (or that was pushed from a lagging clock) sat permanently below
//! the cursor and could never recover — silently skipped data.
//!
//! Post-fix: the transport surfaces every complete batch with its wire
//! batch UUID ("b" tag) at ANY HLC position; `sync_nostr_pull` admits any
//! batch whose UUID is unmarked in `sync_applied_batches`, applying and
//! marking in one transaction. The cursor survives only as an advance-only
//! observability high-water mark.
//!
//! These tests drive the real FFI surface (`sync_nostr_pull`) against an
//! in-process local relay; out-of-band batches are published directly via
//! `sync_nostr` transports holding the SAME identity as the pulling
//! instance, so crafted HLC orderings can replay the audit scenario.

use std::path::PathBuf;
use std::sync::Arc;

use greek_core::sync::{ReviewEvent, SyncBatch};
use uniffi_bindings::GreekCore;

fn tmp_db(tag: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("greek-s6-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), dir.join("greek_app.db").to_string_lossy().into_owned())
}

fn new_core(db: String) -> Arc<GreekCore> {
    GreekCore::new(db, None).unwrap().into()
}

/// Start an in-process local nostr relay on an ephemeral port. The returned
/// `LocalRelay` MUST stay in scope for the relay to keep serving.
async fn local_relay() -> (nostr_relay_builder::LocalRelay, String) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let builder = nostr_relay_builder::RelayBuilder::default()
        .addr(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
        .port(port)
        .rate_limit(nostr_relay_builder::builder::RateLimit {
            max_reqs: 100_000,
            notes_per_minute: 1_000_000,
        });
    let relay = nostr_relay_builder::LocalRelay::new(builder);
    relay.run().await.expect("local relay bind/run");
    let url = relay.url().await.to_string();
    (relay, url.trim_end_matches('/').to_string())
}

/// One valid wire review event (passes every ingress gate in
/// `apply_review_events`: rating 1..=4, canonical `YYYY-MM-DDTHH:MM:SSZ`
/// timestamp, parseable HLC).
fn review(uuid: &str, card: &str, second: u32, rating: u32, hlc_ms: u64, dev: &str) -> ReviewEvent {
    ReviewEvent {
        uuid: uuid.into(),
        card_id: card.into(),
        timestamp: format!("2026-09-01T10:{:02}:{:02}Z", second / 60, second % 60),
        rating,
        elapsed_days: 0.0,
        scheduled_days: 1.0,
        source: "review".into(),
        device_id: dev.into(),
        hlc: format!("{hlc_ms:013}-0000-{dev}-node"),
    }
}

fn batch(dev: &str, to_hlc_ms: u64, reviews: Vec<ReviewEvent>) -> SyncBatch {
    SyncBatch {
        v: greek_core::sync::SYNC_WIRE_VERSION_MAX,
        device_id: dev.into(),
        from_hlc: String::new(),
        to_hlc: format!("{to_hlc_ms:013}-0000-{dev}-node"),
        reviews,
        settings: Vec::new(),
        known_words: Vec::new(),
        lookup_cards: Vec::new(),
        chapter_unlocks: Vec::new(),
    }
}

/// All review uuids the instance currently holds (via a backup export).
fn held_review_uuids(core: &GreekCore) -> Vec<String> {
    let json: serde_json::Value =
        serde_json::from_str(&core.sync_export_backup_json()).expect("export parses");
    let mut uuids: Vec<String> = json["reviews"]
        .as_array()
        .expect("reviews array")
        .iter()
        .map(|r| r["uuid"].as_str().unwrap().to_string())
        .collect();
    uuids.sort();
    uuids
}

/// S6 HEADLINE (audit repro, inverted): complete NEWER batch B applies and
/// marks first (advancing the pull high-water mark past every HLC of A);
/// complete OLDER batch A — to_hlc BELOW the mark — arrives later. Pre-fix
/// A was filtered at the cursor and its rows were permanently lost. Now
/// BOTH merge, and the receiver's surface equals an authoritative replay.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_older_batch_below_pull_highwater_still_merges() {
    let (_relay, url) = local_relay().await;

    // C: the pulling instance.
    let (dir_c, db_c) = tmp_db("hl-c");
    let c = new_core(db_c);
    c.sync_generate_nostr_identity();
    c.sync_set_nostr_relays(url.clone());
    let nsec = c.secret_get("nostr_nsec".into()).expect("identity stored");
    let keys = nostr_sdk::prelude::Keys::parse(&nsec).unwrap();
    let t = sync_nostr::NostrSpoolTransport::connect(keys, &[url.as_str()])
        .await
        .expect("connect");

    // B first: newer HLCs. Publish out-of-band (same identity) as device B.
    let b_reviews: Vec<ReviewEvent> = (0..3u64)
        .map(|i| {
            review(
                &format!("b-uuid-{i}"),
                &format!("card-b{i}"),
                (i * 61 % 3600) as u32,
                3,
                1_800_000_000_000 + i,
                "devb",
            )
        })
        .collect();
    let uuid_set = |rs: &[ReviewEvent]| -> Vec<String> {
        let mut v: Vec<String> = rs.iter().map(|r| r.uuid.clone()).collect();
        v.sort();
        v
    };
    let batch_b = batch("devb", 1_800_000_001_000, b_reviews.clone());
    t.push(&batch_b).await.expect("push B");

    // A second: OLDER HLCs — below B in every respect (the audit trap).
    // Multi-chunk on purpose is unnecessary here; three plain reviews.
    let a_reviews: Vec<ReviewEvent> = (0..2u64)
        .map(|i| {
            review(
                &format!("a-uuid-{i}"),
                &format!("card-a{i}"),
                (i * 61 % 3600) as u32,
                4,
                1_700_000_000_000 + i,
                "deva",
            )
        })
        .collect();

    // Step 1: only B on the spool. C pulls — B applies, B marks, the
    // high-water mark lands at 1800... ABOVE every HLC A will ever have.
    let pull1 = c.sync_nostr_pull().await.expect("pull 1");
    assert_eq!(pull1.reviews_received, 3, "B applies: {pull1:?}");
    assert_eq!(held_review_uuids(&c), uuid_set(&b_reviews));

    // Step 2: A's events hit the relay LATE (incomplete at pull-1 time, a
    // lagging-clock device finally uploading — same failure shape).
    let batch_a = batch("deva", 1_700_000_001_000, a_reviews.clone());
    t.push(&batch_a).await.expect("push A late");

    // Step 3: pre-fix this pull returned ZERO new rows forever (A filtered
    // at the cursor); post-fix A's UUID is unmarked, so A applies at ANY
    // HLC position.
    let pull2 = c.sync_nostr_pull().await.expect("pull 2");
    assert_eq!(
        pull2.reviews_received, 2,
        "A must merge DESPITE sitting below the pull high-water mark: {pull2:?}"
    );

    // Step 4: re-pull — every UUID marked, no double-apply.
    let pull3 = c.sync_nostr_pull().await.expect("pull 3");
    assert_eq!(pull3.reviews_received, 0, "marked batches are skipped: {pull3:?}");

    let mut expect = uuid_set(&a_reviews);
    expect.extend(uuid_set(&b_reviews));
    expect.sort();
    assert_eq!(held_review_uuids(&c), expect, "exactly A ∪ B, once");

    // Authoritative replay: a fresh instance folds A ∪ B in ONE import;
    // its memory-state surface must equal C's row-for-row.
    let (dir_d, db_d) = tmp_db("hl-d");
    let d = new_core(db_d);
    let mut all = batch_a.clone();
    all.reviews.extend(b_reviews.iter().cloned());
    let stats = d
        .sync_import_backup_json(serde_json::to_string(&all).unwrap())
        .unwrap();
    assert_eq!(stats.reviews_received, 5, "authoritative replay: {stats:?}");
    let due_c = c.get_due_cards(1000).unwrap();
    let due_d = d.get_due_cards(1000).unwrap();
    assert_eq!(due_c.len(), 5, "all five reviewed cards fold: {:?} vs {:?}", due_c.len(), due_d.len());
    assert_eq!(due_c.len(), due_d.len());
    for (cc, dc) in due_c.iter().zip(due_d.iter()) {
        assert_eq!(cc.card.id, dc.card.id);
        assert_eq!(cc.stability, dc.stability);
        assert_eq!(cc.difficulty, dc.difficulty);
        assert_eq!(cc.due_date, dc.due_date);
        assert_eq!(cc.reps, dc.reps);
        assert_eq!(cc.lapses, dc.lapses);
    }

    drop((c, d));
    let _ = std::fs::remove_dir_all(&dir_c);
    let _ = std::fs::remove_dir_all(&dir_d);
}

/// S6 RECOVERY: a previously-INCOMPLETE batch (final chunk late) completes
/// on a later fetch and applies EXACTLY ONCE even though the pull
/// high-water mark was meanwhile advanced past its HLCs by complete batch
/// B; a further re-pull does not double-apply.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_late_final_chunk_completes_batch_exactly_once() {
    let (_relay, url) = local_relay().await;

    let (dir_c, db_c) = tmp_db("lc-c");
    let c = new_core(db_c);
    c.sync_generate_nostr_identity();
    c.sync_set_nostr_relays(url.clone());
    let nsec = c.secret_get("nostr_nsec".into()).expect("identity stored");
    let keys = nostr_sdk::prelude::Keys::parse(&nsec).unwrap();
    let keys2 = nostr_sdk::prelude::Keys::parse(&nsec).unwrap();
    let t = sync_nostr::NostrSpoolTransport::connect(keys, &[url.as_str()])
        .await
        .expect("connect");

    // A: OLDER, multi-CHUNK (>> 32 KiB JSON), to_hlc below B's.
    let a_reviews: Vec<ReviewEvent> = (0..300u64)
        .map(|i| {
            review(
                &format!("late-a-uuid-{i}"),
                &format!("card-late-{}", i % 40), // several reviews per card
                (i * 61 % 3600) as u32,
                (i % 4 + 1) as u32,
                1_700_000_000_000 + i,
                "deva",
            )
        })
        .collect();
    let batch_a = batch("deva", 1_700_000_001_000, a_reviews.clone());
    let a_events = sync_nostr::codec::build_chunk_events(&keys2, &batch_a).unwrap();
    assert!(a_events.len() >= 2, "A must be multi-chunk: {}", a_events.len());

    // Publish A's chunks MINUS the final one (final chunk in flight/lost).
    for ev in &a_events[..a_events.len() - 1] {
        t.client.send_event(ev).await.expect("publish partial chunk");
    }
    // B: complete, NEWER — same round, so pull 1 advances the high-water
    // mark above A's HLCs while A is still incomplete.
    let b_reviews: Vec<ReviewEvent> = (0..2u64)
        .map(|i| {
            review(
                &format!("late-b-uuid-{i}"),
                &format!("card-late-b{i}"),
                (i * 61 % 3600) as u32,
                2,
                1_800_000_000_000 + i,
                "devb",
            )
        })
        .collect();
    let batch_b = batch("devb", 1_800_000_001_000, b_reviews.clone());
    t.push(&batch_b).await.expect("push B");

    // Pull 1: B applies+marks; A is partial — tolerated, NOT surfaced, NOT
    // marked (pre-fix this combination was impossible to recover from: the
    // cursor now sits at 1800... > A's to_hlc).
    let pull1 = c.sync_nostr_pull().await.expect("pull 1");
    assert_eq!(pull1.reviews_received, 2, "only B: {pull1:?}");

    // The late final chunk lands: A completes.
    t.client
        .send_event(a_events.last().unwrap())
        .await
        .expect("publish final chunk");

    // Pull 2: A surfaces WITH its batch UUID at an HLC below the mark;
    // unmarked -> applies. Exactly A's rows land.
    let pull2 = c.sync_nostr_pull().await.expect("pull 2");
    assert_eq!(
        pull2.reviews_received,
        a_reviews.len() as u32,
        "completed-late A applies exactly once: {pull2:?}"
    );

    // Pull 3: everything marked -> no double-apply.
    let pull3 = c.sync_nostr_pull().await.expect("pull 3");
    assert_eq!(pull3.reviews_received, 0, "no double-apply: {pull3:?}");

    let mut expect: Vec<String> = a_reviews
        .iter()
        .chain(b_reviews.iter())
        .map(|r| r.uuid.clone())
        .collect();
    expect.sort();
    expect.dedup();
    assert_eq!(held_review_uuids(&c), expect);

    drop(c);
    let _ = std::fs::remove_dir_all(&dir_c);
}
