//! Minimal LOCAL nostr relay for cross-device sync E2E.
//!
//! Speaks NIP-01 over cleartext ws://127.0.0.1:<port> using rust-nostr's
//! official `nostr-relay-builder` 0.43 (same line as the workspace's pinned
//! `nostr-sdk 0.43`). Backed by an in-memory database that enforces standard
//! relay semantics: event id+signature verification, and keep-newest
//! replacement for replaceable/parameterized-replaceable events (so the
//! kind-30078 blinded checkpoints collapse to one per (pubkey, kind, d-tag)).
//! REQ answers all stored events matching the filter (authors / kinds /
//! `#t` / `#d` / any single-letter tag), then sends EOSE; subscriptions stay
//! open with live-push + CLOSE handling, which is a superset of what
//! `NostrSpoolTransport` needs (its `fetch_events` is once-and-close).
//!
//! CLI:
//!   local_relay [--port 7777] [--once-events N] [--max-secs 300]
//!
//! Prints `RELAY_READY 127.0.0.1:<port>` on stdout once the listener is
//! actually bound, and exits cleanly on SIGINT/SIGTERM, after the
//! `--max-secs` deadline, or after `--once-events N` admitted events.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nostr_database::prelude::BoxedFuture;
use nostr_relay_builder::builder::{PolicyResult, RateLimit, WritePolicy};
use nostr_relay_builder::{LocalRelay, RelayBuilder};
use nostr_sdk::prelude::Event;

/// Write policy that admits everything (relay verification still applies)
/// and counts admitted events for `--once-events`.
#[derive(Debug)]
struct CountingPolicy {
    admitted: Arc<AtomicU64>,
}

impl WritePolicy for CountingPolicy {
    fn admit_event<'a>(
        &'a self,
        _event: &'a Event,
        _addr: &'a SocketAddr,
    ) -> BoxedFuture<'a, PolicyResult> {
        self.admitted.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { PolicyResult::Accept })
    }
}

struct Args {
    port: u16,
    once_events: Option<u64>,
    max_secs: u64,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        port: 7777,
        once_events: None,
        max_secs: 300,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => {
                args.port = it
                    .next()
                    .ok_or("--port needs a value")?
                    .parse()
                    .map_err(|e| format!("bad --port: {e}"))?;
            }
            "--once-events" => {
                args.once_events = Some(
                    it.next()
                        .ok_or("--once-events needs a value")?
                        .parse()
                        .map_err(|e| format!("bad --once-events: {e}"))?,
                );
            }
            "--max-secs" => {
                args.max_secs = it
                    .next()
                    .ok_or("--max-secs needs a value")?
                    .parse()
                    .map_err(|e| format!("bad --max-secs: {e}"))?;
            }
            "--help" | "-h" => {
                println!("local_relay [--port 7777] [--once-events N] [--max-secs 300]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg: {other}")),
        }
    }
    Ok(args)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args().map_err(|e| {
        eprintln!("{e}\nusage: local_relay [--port 7777] [--once-events N] [--max-secs 300]");
        e
    })?;

    let admitted = Arc::new(AtomicU64::new(0));
    let builder = RelayBuilder::default()
        .addr(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .port(args.port)
        // Test tooling: don't get in the way of bursty chunk publishes.
        .rate_limit(RateLimit {
            max_reqs: 100_000,
            notes_per_minute: 1_000_000,
        })
        .write_policy(CountingPolicy {
            admitted: admitted.clone(),
        });

    // `run` binds the TCP listener before returning, so this line is only
    // printed once the relay is genuinely accepting connections.
    let relay = LocalRelay::run(builder).await?;
    println!(
        "RELAY_READY {}",
        relay.url().trim_start_matches("ws://")
    );

    // Watch for the --once-events admission quota (parked future if unset).
    let quota = args.once_events;
    let counted = admitted.clone();
    let quota_watch = async move {
        match quota {
            Some(n) => loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                if counted.load(Ordering::SeqCst) >= n {
                    break n;
                }
            },
            None => std::future::pending::<u64>().await,
        }
    };

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    let reason = tokio::select! {
        _ = tokio::signal::ctrl_c() => "SIGINT".to_string(),
        _ = async {
            #[cfg(unix)]
            {
                sigterm.recv().await
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await
            }
        } => "SIGTERM".to_string(),
        _ = tokio::time::sleep(Duration::from_secs(args.max_secs)) => {
            format!("max-secs {} reached", args.max_secs)
        }
        n = quota_watch => {
            // Grace period so in-flight saves/notifications land.
            tokio::time::sleep(Duration::from_millis(500)).await;
            format!("once-events {n} admitted")
        }
    };

    relay.shutdown();
    println!("RELAY_STOP {reason}");
    Ok(())
}
