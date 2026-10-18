//! Wave 7: cross-device sync end-to-end proof.
//!
//! Two `GreekCore` instances on independent data directories stand in for
//! two physical devices ("phone" = A, "desktop" = B) and must converge
//! through every transport-backed path shipped in wave 6b:
//!
//!  1. iroh-docs live sync: pairing via ticket, A -> B pull convergence,
//!     then B -> A reverse (the joined peer dials the original sharer, so
//!     only B needs the ticket — mirrors the loopback spike technique).
//!  2. Backup-file export -> import, including idempotent re-import.
//!  3. nostr pull without a configured identity: clean GreekError, no panic.
//!  4. Settings HLC-LWW: an older-losing write applied at the core merge
//!     surface cannot unlatch a newer gate latch (strict-greater rule).
//!
//! Runs against the crate's rlib (host-side), so it exercises exactly the
//! code paths the Kotlin layer drives over the FFI.

use std::collections::BTreeSet;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use greek_core::sync::Hlc;
use uniffi_bindings::{GreekCore, GreekError};

/// Any single iroh FFI op taking longer than this fails the test instead of
/// hanging CI.
const IROH_OP_TIMEOUT: Duration = Duration::from_secs(60);
/// Upper bound for B-side pushes to appear in A's doc (acceptor-side commit
/// after SyncFinished is asynchronous; see sync-iroh crate test).
const CONVERGENCE_WAIT: Duration = Duration::from_secs(45);

fn tmp_db(tag: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("greek-e2e-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("greek_app.db").to_string_lossy().into_owned();
    (dir, db)
}

/// The real letters track as one JSON array (the same JSON the app passes
/// to `new_with_cards`). Needed because the alphabet gate is fail-open
/// SATISFIED with method "" when no letters deck is loaded — a gate-latch
/// assertion is only meaningful with the deck present (see the wave-4
/// alphabet_gate_integration test for the same fixture choice).
fn letters_json() -> String {
    include_str!("../../../data/cards/letters.json").to_string()
}

fn core_with_letters(db_path: String) -> Arc<GreekCore> {
    GreekCore::new_with_cards(db_path, None, letters_json()).unwrap()
}

/// Parse a `sync_export_backup_json` payload back into the canonical v1
/// wire batch (proves the export is well-formed as well as uses it).
fn export_batch(core: &GreekCore) -> greek_core::SyncBatch {
    let json = core.sync_export_backup_json();
    serde_json::from_str(&json).unwrap_or_else(|e| panic!("export did not parse as SyncBatch v1: {e}\n{json}"))
}

fn uuid_set(batch: &greek_core::SyncBatch) -> BTreeSet<String> {
    batch.reviews.iter().map(|r| r.uuid.clone()).collect()
}

/// Equality of everything a device's SRS surface can show for a deck:
/// same cards due, same FSRS state per card.
fn assert_due_surface_parity(a: &GreekCore, b: &GreekCore, ctx: &str) {
    let da = a.get_due_cards(100).unwrap();
    let db = b.get_due_cards(100).unwrap();
    assert_eq!(da.len(), db.len(), "{ctx}: due-list length mismatch");
    for (ca, cb) in da.iter().zip(db.iter()) {
        assert_eq!(ca.card.id, cb.card.id, "{ctx}: due card order/id");
        assert_eq!(ca.stability, cb.stability, "{ctx} {}: stability", ca.card.id);
        assert_eq!(ca.difficulty, cb.difficulty, "{ctx} {}: difficulty", ca.card.id);
        assert_eq!(ca.due_date, cb.due_date, "{ctx} {}: due_date", ca.card.id);
        assert_eq!(ca.reps, cb.reps, "{ctx} {}: reps", ca.card.id);
        assert_eq!(ca.lapses, cb.lapses, "{ctx} {}: lapses", ca.card.id);
    }
}

/// Chainable result of a bounded-timeout await: panics with context on
/// timeout instead of hanging the whole binary (slow-CI safe: generous
/// but strictly bounded).
async fn must<T, F>(fut: F, what: &str) -> T
where
    F: Future<Output = Result<T, GreekError>>,
{
    match tokio::time::timeout(IROH_OP_TIMEOUT, fut).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => panic!("{what} failed: {e}"),
        Err(_) => panic!("{what} timed out after {IROH_OP_TIMEOUT:?}"),
    }
}

/// TEST 1: two GreekCore instances converge over iroh-docs, both directions.
///
/// A ("phone") seeds 5 real reviews across 3 cards (changing ratings) and
/// latches the alphabet gate through the real check-verdict FFI path.
/// B ("desktop") joins via A's ticket, syncs, and must end up with
/// byte-identical review uuid sets, identical due/memory surface, and the
/// gate latch applied. Then B seeds 2 fresh reviews and A must see them —
/// proving the merge is symmetric, not producer->consumer only.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_iroh_two_instances_converge() {
    let t0 = Instant::now();
    let (dir_a, db_a) = tmp_db("iroh-a");
    let (dir_b, db_b) = tmp_db("iroh-b");

    // -- Device A ("phone"): seed real user history -------------------------
    let a = core_with_letters(db_a);
    for (card, rating) in [
        ("card-1", 3u8),
        ("card-1", 4), // changing rating on the same card
        ("card-2", 2),
        ("card-2", 3),
        ("card-3", 1), // an Again (lapse)
    ] {
        assert!(a.submit_review(card.into(), rating), "A seed {card}");
    }
    // Settings latch via the real alphabet-gate check path (>=22 of >=24).
    assert!(a.record_alphabet_check_result(23, 24));
    let dev_a = a.sync_device_id();

    let seed_a = export_batch(&a);
    assert_eq!(seed_a.reviews.len(), 5, "A must have exactly the seeded reviews");
    let uuids_a = uuid_set(&seed_a);

    // -- Device B ("desktop"): fresh install --------------------------------
    let b = core_with_letters(db_b);
    let dev_b = b.sync_device_id();
    assert_ne!(dev_a, dev_b, "devices must have distinct sync ids");
    assert!(!b.get_alphabet_gate_state().satisfied, "B starts unlatched");

    // -- Pairing + first convergence wave (A -> B) --------------------------
    let ticket = must(a.sync_iroh_ticket(), "A sync_iroh_ticket").await;
    assert!(ticket.starts_with("greeksync://ticket/"), "ticket payload shape: {ticket}");
    must(b.sync_iroh_join(ticket), "B sync_iroh_join").await;

    // A pushes its history into the doc (no peers known yet — harmless);
    // B then pushes nothing, dials A (peer from the ticket), pulls + merges.
    let s_a1 = must(a.sync_iroh_now(), "A sync_iroh_now #1").await;
    assert_eq!(s_a1.reviews_sent, 5, "A's first push sends all 5 events: {s_a1:?}");
    let t_joined = Instant::now();
    let s_b1 = must(b.sync_iroh_now(), "B sync_iroh_now #1").await;
    let join_converge_ms = t_joined.elapsed();

    assert_eq!(s_b1.reviews_received, 5, "B must receive A's 5 events: {s_b1:?}");
    assert!(s_b1.settings_applied >= 2, "B must apply gate latch settings: {s_b1:?}");

    // B converged: identical review uuid set, identical memory/due surface.
    let on_b = export_batch(&b);
    assert_eq!(
        uuid_set(&on_b),
        uuids_a,
        "B's review uuid set must equal A's after first sync"
    );
    assert_due_surface_parity(&a, &b, "after A->B sync");
    assert!(
        b.get_alphabet_gate_state().satisfied,
        "gate latch must propagate to B over iroh"
    );
    assert_eq!(b.get_alphabet_gate_state().method, "check_passed");
    assert!(
        b.sync_paired_devices().iter().any(|d| d.device_id == dev_a),
        "B's paired-device registry must list A"
    );

    // -- Reverse wave (B -> A): B originates, A converges -------------------
    assert!(b.submit_review("card-1".to_string(), 4));
    assert!(b.submit_review("card-4".to_string(), 3));
    let t_phase2 = Instant::now();
    must(b.sync_iroh_now(), "B sync_iroh_now #2").await;

    // B dialed A during its sync round, so B's entries live-replicate into
    // A's doc; A just keeps pulling its doc until the uuid set matches
    // (bounded — the acceptor-side commit is asynchronous).
    let b_full = export_batch(&b);
    let want = uuid_set(&b_full);
    assert_eq!(want.len(), 7, "B now has 5 + its 2 new reviews");
    let deadline = Instant::now() + CONVERGENCE_WAIT;
    loop {
        must(a.sync_iroh_now(), "A sync_iroh_now #N").await;
        if uuid_set(&export_batch(&a)) == want {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "A did not converge to B's 7-event uuid set within {CONVERGENCE_WAIT:?}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let phase2_ms = t_phase2.elapsed();

    // A converged too: superset invariant (A's full set == union, uuid-keyed)
    // and the two devices now show the same memory surface.
    let on_a = export_batch(&a);
    let uuids_a2 = uuid_set(&on_a);
    assert!(uuids_a2.is_superset(&uuids_a), "A must keep its own history");
    assert_eq!(uuids_a2.len(), 7, "A must hold A(5) + B(2) events");
    assert_due_surface_parity(&a, &b, "after B->A sync");
    assert!(
        a.sync_paired_devices().iter().any(|d| d.device_id == dev_b),
        "A's paired-device registry must learn B from remote events"
    );

    println!(
        "TIMING test1: A->B join+converge {join_converge_ms:?}, B->A converge {phase2_ms:?}, total {:?}",
        t0.elapsed()
    );

    drop(a);
    drop(b);
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

/// TEST 2: the backup-file transport — export JSON from A, import on B,
/// counts/state equal, re-import strictly idempotent at the FFI surface.
#[test]
fn e2e_backup_file_roundtrip() {
    let (dir_a, db_a) = tmp_db("bak-a");
    let (dir_b, db_b) = tmp_db("bak-b");

    let a = core_with_letters(db_a);
    for (card, rating) in [("card-1", 3u8), ("card-2", 4), ("card-2", 2), ("card-3", 1)] {
        assert!(a.submit_review(card.into(), rating));
    }
    assert!(a.record_alphabet_check_result(24, 24));

    let json = a.sync_export_backup_json();
    let batch: greek_core::SyncBatch = serde_json::from_str(&json).unwrap();
    assert_eq!(batch.v, 1);
    assert_eq!(batch.reviews.len(), 4);
    assert_eq!(batch.device_id, a.sync_device_id());

    let b = core_with_letters(db_b);
    assert!(!b.get_alphabet_gate_state().satisfied, "B starts unlatched");
    let stats = b.sync_import_backup_json(json.clone()).unwrap();
    assert_eq!(stats.reviews_received, 4, "first import applies everything: {stats:?}");
    assert!(stats.settings_applied >= 2, "gate latch settings applied: {stats:?}");

    // State equality: identical uuid sets (uuid-keyed history view),
    // identical due/memory surface, gate latch arrived.
    let on_b = export_batch(&b);
    assert_eq!(uuid_set(&on_b), uuid_set(&batch));
    assert_due_surface_parity(&a, &b, "after backup import");
    assert!(b.get_alphabet_gate_state().satisfied);

    // Idempotency at the FFI surface: replaying the identical payload
    // inserts zero rows and applies zero settings (uuid-dedup + HLC-LWW).
    let again = b.sync_import_backup_json(json).unwrap();
    assert_eq!(again.reviews_received, 0, "replay must be a no-op: {again:?}");
    assert_eq!(again.settings_applied, 0, "same-HLC settings lose: {again:?}");

    drop(a);
    drop(b);
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

/// TEST 3: nostr pull on a device that never configured identity/relays is
/// a clean, typed GreekError::Sync — never a panic or a partial crash.
#[tokio::test]
async fn e2e_nostr_pull_unconfigured_is_clean_error() {
    let (dir, db) = tmp_db("nostr");
    let core = GreekCore::new(db, None).unwrap();

    // Not configured at all: this is the pristine-install state.
    let cfg = core.sync_nostr_config();
    assert!(!cfg.has_identity);

    let err = core.sync_nostr_pull().await.unwrap_err();
    match &err {
        GreekError::Sync(msg) => {
            assert!(
                msg.contains("identity not configured"),
                "error must explain the missing identity, got: {msg}"
            );
            assert!(
                msg.contains("sync_generate_nostr_identity"),
                "error should name the remedy, got: {msg}"
            );
        }
        other => panic!("expected GreekError::Sync, got {other}"),
    }
    // Instance is still healthy afterwards (the error was clean, not poison).
    assert!(core.submit_review("card-1".into(), 3));

    drop(core);
    let _ = std::fs::remove_dir_all(&dir);
}

/// TEST 4: settings HLC-LWW strict-greater rule, cross-device flavored.
/// A latches the alphabet gate at T1; B imports it (now satisfied). An
/// *older*-HLC write for the same key — as if queued on a third device with
/// a weird clock — must NOT unlatch B. A strictly-newer write DOES win.
#[test]
fn e2e_settings_lww_cross_device() {
    let (dir_a, db_a) = tmp_db("lww-a");
    let (dir_b, db_b) = tmp_db("lww-b");

    let a = core_with_letters(db_a);
    assert!(a.record_alphabet_check_result(22, 24)); // pass at the floor
    let t1_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let b = core_with_letters(db_b.clone());
    assert!(!b.get_alphabet_gate_state().satisfied, "B starts unlatched");
    let stats = b.sync_import_backup_json(a.sync_export_backup_json()).unwrap();
    assert!(stats.settings_applied >= 2);
    assert!(b.get_alphabet_gate_state().satisfied);
    assert_eq!(b.get_alphabet_gate_state().method, "check_passed");
    drop(b);
    drop(a);

    // FFI-adjacent core-level merge on B's on-disk database: an older write.
    let older = Hlc {
        ms: t1_ms - 3_600_000,
        counter: 0,
        node: "olddev01".to_string(),
    }
    .to_string();
    let db = greek_core::Database::open(&db_b, None).unwrap();
    let applied = greek_core::sync::apply_settings(
        &db,
        &[("alphabet_gate".to_string(), "organic".to_string(), older)],
    )
    .unwrap();
    assert_eq!(applied, 0, "strictly-older HLC must lose the LWW race");

    // ...while a strictly-newer one wins (both directions of the rule).
    let newer = Hlc {
        ms: t1_ms + 3_600_000,
        counter: 0,
        node: "newdev01".to_string(),
    }
    .to_string();
    let applied = greek_core::sync::apply_settings(
        &db,
        &[("alphabet_gate".to_string(), "organic".to_string(), newer)],
    )
    .unwrap();
    assert_eq!(applied, 1, "strictly-newer HLC must win");
    drop(db);

    // Reopen B at FFI level: after ONLY the older (losing) write had been
    // applied... to check without the newer write having landed, redo a
    // clean pass on a second db where we apply ONLY the loser.
    let (_dir_c, db_c) = tmp_db("lww-c");
    let db3 = greek_core::Database::open(&db_c, None).unwrap();
    greek_core::Curriculum::record_alphabet_gate(&db3, greek_core::AlphabetGateMethod::CheckPassed)
        .unwrap();
    let loser = Hlc {
        ms: 1_000_000_000_000, // 2001 — far in the past
        counter: 0,
        node: "ancient1".to_string(),
    }
    .to_string();
    let applied = greek_core::sync::apply_settings(
        &db3,
        &[("alphabet_gate".to_string(), "organic".to_string(), loser)],
    )
    .unwrap();
    assert_eq!(applied, 0);
    drop(db3);
    let c = core_with_letters(db_c);
    let gate = c.get_alphabet_gate_state();
    assert!(gate.satisfied, "stale cross-device write must not unlatch the gate");
    assert_eq!(gate.method, "check_passed");

    drop(c);
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}
