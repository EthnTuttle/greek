//! Live push -> pull roundtrip against the LOCAL relay binary
//! (`examples/local_relay.rs`, nostr-relay-builder based). Self-contained:
//! the test spawns the relay itself on an ephemeral port, waits for the
//! `RELAY_READY` line, drives `NostrSpoolTransport`, then kills the child.
//! Not #[ignore]d — runs green in the normal workspace test suite.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nostr_sdk::prelude::*;
use sync_nostr::codec::{
    decode_checkpoint_event, synthetic_batch, KIND_BATCH_CHUNK, KIND_CHECKPOINT, TAG_TOPIC,
};
use sync_nostr::NostrSpoolTransport;

/// RAII guard so a panicking test never leaks a relay child process.
struct RelayChild(Child);

impl Drop for RelayChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn example_binary() -> PathBuf {
    // current_exe = <target>/debug/deps/live_local_relay-<hash>
    let exe = std::env::current_exe().expect("current_exe");
    let debug_dir = exe.parent().and_then(|p| p.parent()).expect("debug dir");
    debug_dir.join("examples").join("local_relay")
}

fn ensure_example_built() -> PathBuf {
    let bin = example_binary();
    if bin.exists() {
        return bin;
    }
    // Not built yet (e.g. `cargo test` without examples): build it. cwd of
    // tests is the crate root; env (CARGO_TARGET_DIR etc.) is inherited.
    let status = Command::new("cargo")
        .args(["build", "--example", "local_relay"])
        .status()
        .expect("spawn cargo build");
    assert!(status.success(), "cargo build --example local_relay failed");
    assert!(bin.exists(), "example binary still missing at {bin:?}");
    bin
}

fn spawn_relay() -> (RelayChild, u16) {
    spawn_relay_with(&[])
}

fn spawn_relay_with(extra_args: &[&str]) -> (RelayChild, u16) {
    let bin = ensure_example_built();
    // Reserve an ephemeral port, release it, hand it to the relay.
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral")
        .local_addr()
        .unwrap()
        .port();

    let mut cmd = Command::new(bin);
    cmd.args(["--port", &port.to_string(), "--max-secs", "120"])
        .args(extra_args);
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn local_relay");

    let stdout = child.stdout.take().expect("stdout piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(l) => {
                    println!("relay: {l}");
                    let _ = tx.send(l);
                }
                Err(_) => break,
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let line = rx
            .recv_timeout(Duration::from_millis(500))
            .unwrap_or_default();
        if let Some(rest) = line.strip_prefix("RELAY_READY ") {
            let (host, p): (&str, &str) = rest.split_once(':').expect("host:port");
            assert_eq!(host, "127.0.0.1");
            assert_eq!(p.parse::<u16>().unwrap(), port);
            break;
        }
        assert!(Instant::now() < deadline, "relay did not signal RELAY_READY");
    }
    (RelayChild(child), port)
}

#[tokio::test]
async fn live_local_relay_push_pull_roundtrip() {
    let (_relay, port) = spawn_relay();
    let url = format!("ws://127.0.0.1:{port}");

    let keys = Keys::generate();
    let t = NostrSpoolTransport::connect(keys.clone(), &[url.as_str()])
        .await
        .expect("connect to local relay");

    // Multi-chunk push: 800 reviews ~= 100+ KiB JSON => several 48 KiB chunks.
    let big = synthetic_batch("dev-e2e", 800);
    let chunk_count = sync_nostr::codec::build_chunk_events(&keys, &big)
        .unwrap()
        .len();
    assert!(chunk_count >= 2, "test payload must be multi-chunk");
    t.push(&big).await.expect("push big batch");
    println!("pushed multi-chunk batch ({chunk_count} chunks)");

    // Second, small push => second checkpoint replaces the first.
    let small = synthetic_batch("dev-e2e-b", 5);
    t.push(&small).await.expect("push small batch");

    // Roundtrip equality for both batches.
    let fetched = t.fetch_since(None).await.expect("fetch all");
    assert!(fetched.contains(&big), "multi-chunk batch missing after pull");
    assert!(fetched.contains(&small), "small batch missing after pull");
    println!("pull roundtrip equality OK ({} batches)", fetched.len());

    // since_hlc filter still works.
    let filtered = t.fetch_since(Some(&small.to_hlc)).await.expect("fetch since");
    assert!(filtered.iter().all(|b| b.to_hlc > small.to_hlc));
    assert!(!filtered.contains(&small));

    // Checkpoint visibility: exactly ONE kind-30078 exists (both pushes used
    // the same blinded d-tag, so the parameterized-replaceable event must
    // collapse to the newest), and it decodes to the LAST pushed batch.
    let cp_filter = Filter::new()
        .author(keys.public_key())
        .kind(Kind::Custom(KIND_CHECKPOINT))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::T), TAG_TOPIC);
    let cps = t
        .client
        .fetch_events(cp_filter, Duration::from_secs(5))
        .await
        .expect("checkpoint fetch");
    assert_eq!(
        cps.len(),
        1,
        "parameterized-replaceable checkpoint must keep newest only"
    );
    let cp = decode_checkpoint_event(&keys, cps.first().unwrap()).expect("decode checkpoint");
    assert_eq!(cp.device_id, small.device_id);
    assert_eq!(cp.last_to_hlc, small.to_hlc);
    println!("checkpoint visible & newest-only OK: {cp:?}");

    // Sanity: the chunk kind is what it is (guards against constants drift).
    assert_eq!(KIND_BATCH_CHUNK, 1078);

    t.client.disconnect().await;
}

/// `--once-events N` must make the relay exit on its own after admitting
/// N events. A 5-review batch serializes to <48 KiB => 1 chunk + 1
/// checkpoint = exactly 2 admitted events.
#[tokio::test]
async fn live_local_relay_once_events_quota() {
    let (mut relay, port) = spawn_relay_with(&["--once-events", "2"]);
    let url = format!("ws://127.0.0.1:{port}");

    let keys = Keys::generate();
    let t = NostrSpoolTransport::connect(keys.clone(), &[url.as_str()])
        .await
        .expect("connect");
    let small = synthetic_batch("dev-quota", 5);
    assert_eq!(
        sync_nostr::codec::build_chunk_events(&keys, &small)
            .unwrap()
            .len(),
        1,
        "test assumes exactly one chunk per push"
    );
    t.push(&small).await.expect("push");

    // Relay should self-terminate shortly after the 2nd admission.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = relay.0.try_wait().expect("try_wait") {
            println!("relay exited by itself: {status}");
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "relay did not exit after quota");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    std::mem::forget(relay); // already reaped; don't kill in Drop
    t.client.disconnect().await;
}
