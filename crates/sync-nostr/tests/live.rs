//! Live relay probes and roundtrip. All tests here are #[ignore]; run with:
//!   cargo test -p sync-nostr --test live -- --ignored --nocapture --test-threads=1

use std::time::{Duration, Instant};

use nostr_sdk::prelude::*;
use sync_nostr::codec::synthetic_batch;
use sync_nostr::NostrSpoolTransport;

const DAMUS: &str = "wss://relay.damus.io";
// Extra public relays used as fallback when damus rate-limits a fresh key.
const FALLBACK_RELAYS: &[&str] = &["wss://nos.lol", "wss://offchain.pub"];
const JUMBLE: &str = "wss://jumble.virginiafreedom.tech";
const GIT: &str = "wss://git.virginiafreedom.tech";

/// Probe: attempt a real nostr WS connect, 5s timeout. Expects FAILURE on the
/// jumble/git hosts (they serve static pages, not relays) but records the
/// verbatim outcome either way.
async fn probe(url: &str) {
    println!("=== PROBE {url} ===");
    let keys = Keys::generate();
    let client = Client::new(keys);
    client.add_relay(url).await.expect("add_relay");
    let start = Instant::now();
    let connect_fut = async {
        client.connect().await;
        // Give the pool a moment to establish; then check status.
        tokio::time::sleep(Duration::from_secs(4)).await;
        client.relay(url).await.map(|r| r.status())
    };
    match tokio::time::timeout(Duration::from_secs(5), connect_fut).await {
        Ok(Ok(status)) => println!(
            "probe {url}: connected status={status} in {:?}",
            start.elapsed()
        ),
        Ok(Err(e)) => println!("probe {url}: FAILED after {:?}: {e}", start.elapsed()),
        Err(_) => println!("probe {url}: TIMEOUT after 5s (no successful WS upgrade)"),
    }
    // Also try fetching one event to prove relay-ness (EOSE behavior).
    let filter = Filter::new().limit(1);
    match tokio::time::timeout(
        Duration::from_secs(5),
        client.fetch_events(filter, Duration::from_secs(4)),
    )
    .await
    {
        Ok(Ok(events)) => println!("probe {url}: fetch ok, {} event(s)", events.len()),
        Ok(Err(e)) => println!("probe {url}: fetch FAILED: {e}"),
        Err(_) => println!("probe {url}: fetch TIMEOUT (no EOSE)"),
    }
    client.disconnect().await;
}

#[tokio::test]
#[ignore]
async fn probe_jumble_host() {
    probe(JUMBLE).await;
}

#[tokio::test]
#[ignore]
async fn probe_git_host() {
    probe(GIT).await;
}

/// Full push -> fetch roundtrip against wss://relay.damus.io with fresh keys.
#[tokio::test]
#[ignore]
async fn live_damus_roundtrip() {
    let keys = Keys::generate();
    println!("pubkey: {}", keys.public_key().to_hex());
    let mut relays = vec![DAMUS];
    relays.extend_from_slice(FALLBACK_RELAYS);
    let t = NostrSpoolTransport::connect(keys, &relays)
        .await
        .expect("connect");

    async fn push_paced(t: &NostrSpoolTransport, batch: &sync_nostr::SyncBatch) {
        for attempt in 0..12 {
            match t.push(batch).await {
                Ok(()) => return,
                Err(e) if format!("{e}").contains("rate-limited") => {
                    println!("push rate-limited (attempt {attempt}); backing off 15s");
                    tokio::time::sleep(Duration::from_secs(15)).await;
                }
                Err(e) => panic!("push failed: {e}"),
            }
        }
        panic!("push still rate-limited after 12 attempts");
    }

    // Small batch.
    let b1 = synthetic_batch("dev-live-1", 5);
    let small_json = serde_json::to_vec(&b1).unwrap().len();
    let start = Instant::now();
    push_paced(&t, &b1).await;
    println!("pushed small batch ({} bytes json) in {:?}", small_json, start.elapsed());

    // ~200KiB batch to exercise chunking live. Public relays rate-limit
    // bursts ("rate-limited: you are noting too much" observed on damus for
    // ~140 events in <1s from a fresh key), so pace chunk publishes.
    let b2 = synthetic_batch("dev-live-1", 1500);
    let big_json = serde_json::to_vec(&b2).unwrap().len();
    let start = Instant::now();
    let events = sync_nostr::codec::build_chunk_events(&t.keys, &b2).unwrap();
    let mut accepted = 0usize;
    for ev in &events {
        loop {
            match t.client.send_event(ev).await {
                Ok(out) if !out.success.is_empty() => {
                    accepted += 1;
                    break;
                }
                Ok(out) => {
                    let msg = format!("{:?}", out.failed);
                    assert!(msg.contains("rate") || msg.contains("too much"), "{msg}");
                    println!("rate-limited; backing off 10s...");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
                Err(e) => panic!("publish error: {e}"),
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    println!(
        "pushed big batch ({} bytes json, {} chunks, {} accepted) in {:?}",
        big_json,
        events.len(),
        accepted,
        start.elapsed()
    );
    // Checkpoint via normal path (paced).
    push_paced(&t, &synthetic_batch("dev-live-1", 1)).await;

    let start = Instant::now();
    let all = t.fetch_since(None).await.expect("fetch all");
    println!(
        "fetched {} batch(es) in {:?}",
        all.len(),
        start.elapsed()
    );
    assert!(all.contains(&b1), "small batch missing");
    assert!(all.contains(&b2), "big batch missing");

    // since filter: b1.to_hlc sorts before b2... they share hlc pattern from
    // synthetic_batch; filter using b1.to_hlc should yield only batches with
    // strictly greater to_hlc (none here unless to_hlc differs).
    let filtered = t.fetch_since(Some(&b1.to_hlc)).await.expect("fetch since");
    assert!(filtered.iter().all(|b| b.to_hlc > b1.to_hlc));
    println!("fetch_since filter OK: {} batch(es) after", filtered.len());

    t.client.disconnect().await;
}
