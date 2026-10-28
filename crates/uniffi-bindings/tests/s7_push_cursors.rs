//! S7 regression tests: destination-scoped push cursors + import rewind.
//!
//! The S7 bug: outbound selection was `hlc > last_push` under ONE cursor per
//! transport, so (a) history imported AFTER a push whose HLCs predated the
//! cursor never reached that destination, and (b) switching relays /
//! identity / notebook populated only the recent tail instead of full
//! history. These tests drive the real FFI surface:
//!
//!  - nostr: `sync_nostr_push` / `sync_nostr_pull` against in-process local
//!    relays (`nostr-relay-builder`, same 0.44 line as the transport), plus
//!    `sync_import_backup_json` for the import-rewind probe.
//!  - iroh: `sync_iroh_now` across two notebooks in loopback mode.
//!
//! Runs against the crate's rlib (host-side); the Kotlin layer drives the
//! same methods over the FFI.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uniffi_bindings::GreekCore;

fn tmp_db(tag: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("greek-s7-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), dir.join("greek_app.db").to_string_lossy().into_owned())
}

fn new_core(db: String) -> Arc<GreekCore> {
    GreekCore::new(db, None).unwrap().into()
}

/// Small real-time gap so reviews/pushes from two instances order by wall
/// clock across the HLC millisecond boundary.
fn clock_gap() {
    std::thread::sleep(Duration::from_millis(10));
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
        // Test tooling: stay out of the way of bursty chunk publishes.
        .rate_limit(nostr_relay_builder::builder::RateLimit {
            max_reqs: 100_000,
            notes_per_minute: 1_000_000,
        });
    let relay = nostr_relay_builder::LocalRelay::new(builder);
    relay.run().await.expect("local relay bind/run");
    let url = relay.url().await.to_string();
    (relay, url.trim_end_matches('/').to_string())
}

/// S7-a (import rewind): rows merged from a backup AFTER a push, whose HLCs
/// PREDATE the push cursor, must still be delivered on the next push —
/// and prove they physically land, by pulling them with a third instance
/// holding the same identity (the inverted probe: pre-fix this push sends
/// ZERO rows and C stays empty).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_import_after_push_redelivers_old_rows() {
    let (_r1, url1) = local_relay().await;

    // B's history comes FIRST so its HLCs predate A's push cursor below.
    let (dir_b, db_b) = tmp_db("import-b");
    let b = new_core(db_b);
    assert!(b.submit_review("card-b1".into(), 3));
    assert!(b.submit_review("card-b2".into(), 4));
    let backup_b = b.sync_export_backup_json();
    clock_gap();

    // A pushes its own review to R1, advancing its cursor past B's HLCs.
    let (dir_a, db_a) = tmp_db("import-a");
    let a = new_core(db_a);
    a.sync_generate_nostr_identity();
    a.sync_set_nostr_relays(url1.clone());
    assert!(a.submit_review("card-a1".into(), 3));
    clock_gap();
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 1, "A's first push: {s:?}");
    // Baseline cursor behavior: an immediate re-push sends nothing.
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 0, "nothing new must not re-send: {s:?}");

    // Import B's OLDER backup after the push (S7-a trigger).
    let imp = a.sync_import_backup_json(backup_b).unwrap();
    assert_eq!(imp.reviews_received, 2, "import: {imp:?}");

    // The next push must re-ship everything below the old cursor:
    // B's 2 merged rows + A's own 1 (also above the rewound cursor).
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(
        s.reviews_sent, 3,
        "import rewind must re-deliver the merged history: {s:?}"
    );
    // Afterwards the cursor is whole again.
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 0, "{s:?}");

    // Physical delivery proof: C (fresh device, SAME identity, SAME relay)
    // pulls and ends up with all 3 distinct rows.
    let (dir_c, db_c) = tmp_db("import-c");
    let c = new_core(db_c);
    let nsec = a.secret_get("nostr_nsec".into()).expect("A has an identity");
    c.sync_set_nostr_nsec(nsec).unwrap();
    c.sync_set_nostr_relays(url1);
    let got = c.sync_nostr_pull().await.unwrap();
    assert_eq!(
        got.reviews_received, 3,
        "C must receive the full history incl. the imported rows: {got:?}"
    );

    drop((a, b, c));
    for dir in [dir_a, dir_b, dir_c] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// S7-b (nostr destination change): a different relay set is a different
/// destination — first push to it sends FULL history; switching back resumes
/// the previous cursor without a redundant full resend; regenerating the
/// identity is likewise a new destination.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_relay_or_identity_change_rescopes_cursor() {
    let (_r1, url1) = local_relay().await;
    let (_r2, url2) = local_relay().await;

    let (dir_a, db_a) = tmp_db("sw-a");
    let a = new_core(db_a);
    a.sync_generate_nostr_identity();
    let nsec_v1 = a.secret_get("nostr_nsec".into()).unwrap();
    a.sync_set_nostr_relays(url1.clone());
    assert!(a.submit_review("card-1".into(), 3));
    assert!(a.submit_review("card-2".into(), 4));

    // Destination (R1, id_v1): full push, then nothing.
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 2, "{s:?}");
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 0, "{s:?}");

    // NEW relay set → new destination → FULL history again.
    a.sync_set_nostr_relays(url2.clone());
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 2, "new relay must get FULL history: {s:?}");

    // CSV cosmetic variance (whitespace / trailing comma / duplicate) must
    // NOT fork the scope: same normalized destination, cursor kept.
    a.sync_set_nostr_relays(format!(" {url2} ,\n{url2},"));
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 0, "normalized same set keeps cursor: {s:?}");

    // Incremental on R2 works.
    assert!(a.submit_review("card-3".into(), 2));
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 1, "{s:?}");

    // Back to R1: resume R1's cursor — only the review R1 never saw.
    a.sync_set_nostr_relays(url1.clone());
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(
        s.reviews_sent, 1,
        "switch-back must resume the old cursor, not resend: {s:?}"
    );

    // New identity → new destination → FULL history again.
    a.sync_generate_nostr_identity();
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 3, "new identity gets FULL history: {s:?}");

    // Restore the old identity: (R1, id_v1) resumes its own cursor.
    a.sync_set_nostr_nsec(nsec_v1).unwrap();
    let s = a.sync_nostr_push().await.unwrap();
    assert_eq!(s.reviews_sent, 0, "identity switch-back resumes: {s:?}");

    drop(a);
    let _ = std::fs::remove_dir_all(dir_a);
}

/// S7 (iroh destination change): joining a different notebook switches the
/// namespace, hence the cursor scope — the joined destination gets a FULL
/// push, while the (no-longer-active) scope's cursor is preserved.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_iroh_namespace_change_full_push() {
    // Hermetic loopback: no n0 relay / DNS discovery (see e2e_two_device_sync).
    std::env::set_var("GREEK_IROH_NET", "loopback");
    async fn must<T>(
        fut: impl std::future::Future<Output = Result<T, uniffi_bindings::GreekError>>,
        what: &str,
    ) -> T {
        tokio::time::timeout(Duration::from_secs(90), fut)
            .await
            .unwrap_or_else(|_| panic!("{what} timed out"))
            .unwrap_or_else(|e| panic!("{what} failed: {e}"))
    }

    let (dir_a, db_a) = tmp_db("iroh-a");
    let (dir_b, db_b) = tmp_db("iroh-b");
    let a = new_core(db_a);
    let b = new_core(db_b);
    assert!(a.submit_review("x1".into(), 3));
    assert!(a.submit_review("x2".into(), 3));

    // Namespace 1 (A's own notebook): first round pushes everything...
    must(a.sync_iroh_ticket(), "A ticket (ns1)").await;
    let s = must(a.sync_iroh_now(), "A sync ns1 #1").await;
    assert_eq!(s.reviews_sent, 2, "first push to ns1: {s:?}");
    // ...and a second round sends nothing (the scope's cursor advanced).
    let s = must(a.sync_iroh_now(), "A sync ns1 #2").await;
    assert_eq!(s.reviews_sent, 0, "cursor must suppress a resend: {s:?}");

    // Namespace 2: B's notebook. A joins — destination change means the new
    // scope has NO cursor, so the joined notebook gets A's FULL history.
    let ticket_b = must(b.sync_iroh_ticket(), "B ticket (ns2)").await;
    must(a.sync_iroh_join(ticket_b), "A joins ns2").await;
    let s = must(a.sync_iroh_now(), "A sync ns2 #1").await;
    assert_eq!(
        s.reviews_sent, 2,
        "joined notebook must receive FULL history: {s:?}"
    );
    // Incremental within ns2 afterwards.
    let s = must(a.sync_iroh_now(), "A sync ns2 #2").await;
    assert_eq!(s.reviews_sent, 0, "{s:?}");

    drop((a, b));
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
}
