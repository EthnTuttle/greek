//! Integration test for the exported sync surface of GreekCore:
//!  - backup export → fresh instance import → identical due/memory state
//!  - strict wire-version validation on import
//!  - nostr push/pull with an unconfigured identity → clear error, no panic
//!
//! Runs against the crate's rlib (host-side); Kotlin sees the same methods
//! over the FFI.

use uniffi_bindings::{GreekCore, GreekError};

fn tmp_db_path(tag: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("greek-uniffi-sync-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("greek_app.db");
    (dir, path.to_string_lossy().into_owned())
}

#[test]
fn backup_export_import_roundtrip_converges_state() {
    let (dir_a, db_a) = tmp_db_path("a");
    let (dir_b, db_b) = tmp_db_path("b");

    let a = GreekCore::new(db_a, None).unwrap();
    assert!(a.submit_review("card-1".into(), 3));
    assert!(a.submit_review("card-1".into(), 2));
    assert!(a.submit_review("card-2".into(), 4));
    assert!(a.submit_review("card-3".into(), 1));
    // A sync-tracked setting too (alphabet gate latch).
    assert!(a.declare_alphabet_known());

    let json = a.sync_export_backup_json();
    assert!(json.contains("\"v\":1"), "backup must be wire v1: {json}");

    let b = GreekCore::new(db_b, None).unwrap();
    let stats = b.sync_import_backup_json(json).unwrap();
    assert_eq!(stats.reviews_received, 4, "stats: {stats:?}");

    // Import is idempotent: replaying yields zero new rows.
    let stats2 = b
        .sync_import_backup_json(a.sync_export_backup_json())
        .unwrap();
    assert_eq!(stats2.reviews_received, 0);

    // Due-card + memory-state surface must match the exporter exactly.
    let due_a = a.get_due_cards(100).unwrap();
    let due_b = b.get_due_cards(100).unwrap();
    assert_eq!(due_a.len(), due_b.len());
    for (ca, cb) in due_a.iter().zip(due_b.iter()) {
        assert_eq!(ca.card.id, cb.card.id);
        assert_eq!(ca.stability, cb.stability);
        assert_eq!(ca.difficulty, cb.difficulty);
        assert_eq!(ca.due_date, cb.due_date);
        assert_eq!(ca.reps, cb.reps);
        assert_eq!(ca.lapses, cb.lapses);
    }
    // Settings travelled: the alphabet gate latch arrived on B.
    assert!(b.get_alphabet_gate_state().satisfied);
    // Device ids are distinct and stable-looking.
    assert_ne!(a.sync_device_id(), b.sync_device_id());

    drop(a);
    drop(b);
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

#[test]
fn import_rejects_unsupported_wire_version() {
    let (dir, db) = tmp_db_path("ver");
    let core = GreekCore::new(db, None).unwrap();
    let err = core
        .sync_import_backup_json(r#"{"v":2,"device_id":"x","from_hlc":"","to_hlc":"","reviews":[],"settings":[]}"#.into())
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("version"), "expected version error, got: {msg}");
    let err2 = core.sync_import_backup_json("not json".into()).unwrap_err();
    assert!(err2.to_string().contains("JSON"), "got: {err2}");
    drop(core);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nostr_unconfigured_identity_gives_clear_error() {
    let (dir, db) = tmp_db_path("nostr");
    let core = GreekCore::new(db, None).unwrap();
    // Config view: default relay seed, no identity.
    let cfg = core.sync_nostr_config();
    assert_eq!(cfg.relays_csv, "wss://chat.virginiafreedom.tech");
    assert!(!cfg.has_identity);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let err = rt.block_on(core.sync_nostr_push()).unwrap_err();
    match &err {
        GreekError::Sync(msg) => {
            assert!(msg.contains("identity not configured"), "got: {msg}");
        }
        other => panic!("expected Sync error, got {other}"),
    }
    let err = rt.block_on(core.sync_nostr_pull()).unwrap_err();
    assert!(err.to_string().contains("identity not configured"));

    // Setting a bogus secret fails validation; a generated one works.
    let err = core.sync_set_nostr_nsec("bogus".into()).unwrap_err();
    assert!(err.to_string().contains("valid nostr secret key"));
    let npub_hex = core.sync_generate_nostr_identity();
    assert_eq!(npub_hex.len(), 64);
    assert!(core.sync_nostr_config().has_identity);

    drop(core);
    let _ = std::fs::remove_dir_all(&dir);
}
