//! Contract smoke tests: command bodies produce the frozen JSON shapes
//! against a throwaway data dir (no window needed).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use greek_desktop::{commands, state};
use greek_desktop::state::AppState;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn fresh_state() -> AppState {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "greek-desktop-test-{}-{}-{}",
        std::process::id(),
        n,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    // Tests never set GREEK_REPO_ASSETS, so no morphology gets copied in:
    // dict_search must degrade gracefully to [].
    AppState::new(dir).expect("AppState::new")
}

#[test]
fn build_rs_card_merge_total_is_1183() {
    let cards: serde_json::Value =
        serde_json::from_str(state::CARDS_JSON).expect("merged cards.json parses");
    assert_eq!(
        cards.as_array().map(|a| a.len()),
        Some(1183),
        "merged deck must contain all 1183 cards"
    );
}

#[test]
fn home_stats_shape() {
    let s = fresh_state();
    let v: serde_json::Value =
        serde_json::from_str(&commands::home_stats(&s.core).unwrap()).unwrap();
    assert!(v["dueToday"].is_number());
    assert!(v["newToday"].is_number());
    assert_eq!(v["newCap"], 10, "daily new-card cap from the core");
    assert!(v["streakDays"].is_number());
    assert!(v["practiceCorrect"].is_number());
    assert!(v["practiceTotal"].is_number());
    // Fresh install: gate unsatisfied (letters deck loaded, no progress).
    assert_eq!(v["gateSatisfied"], false);
    assert_eq!(v["lettersChaptersMet"], 0);
}

#[test]
fn review_session_fresh_install_serves_letters() {
    let s = fresh_state();
    // Fresh install: alphabet gate unmet, so only the letters deck can feed
    // the queue (core-side gate enforcement). There must be a card.
    let v = commands::review_current(&s).unwrap();
    let card = v["card"].clone();
    assert!(card.is_object(), "expected a first card, got {v}");
    assert_eq!(v["remaining"].as_u64().unwrap() > 0, true);
    assert_eq!(v["isNew"], true);
    let previews = v["previews"].as_array().unwrap();
    assert_eq!(previews.len(), 4, "Again/Hard/Good/Easy labels");
    assert!(previews.iter().all(|p| p.as_str().is_some_and(|t| {
        t.ends_with('m') || t.ends_with('h') || t.ends_with('d')
    })));
    std::fs::remove_dir_all(&s.data_dir).ok();
}

#[test]
fn review_rate_pops_and_validates() {
    let s = fresh_state();
    let first = commands::review_current(&s).unwrap();
    let remaining0 = first["remaining"].as_u64().unwrap();

    // Invalid rating is rejected without consuming the card.
    assert!(commands::review_rate(&s, 0).is_err());
    assert!(commands::review_rate(&s, 5).is_err());

    let next = commands::review_rate(&s, 3).unwrap();
    assert_eq!(next["remaining"].as_u64().unwrap() + 1, remaining0);

    // Drain the session; the final payload is the completion shape and
    // rating beyond that errors.
    loop {
        let cur = commands::review_rate(&s, 3).unwrap();
        if cur["remaining"].as_u64().unwrap() == 0 {
            assert!(cur["card"].is_null());
            break;
        }
    }
    assert!(commands::review_rate(&s, 3).is_err());
    std::fs::remove_dir_all(&s.data_dir).ok();
}

#[test]
fn curriculum_shape() {
    let s = fresh_state();
    let v: serde_json::Value =
        serde_json::from_str(&commands::curriculum(&s.core).unwrap()).unwrap();
    assert_eq!(v["gateNotMet"], true, "fresh install: gate unmet");
    let books = v["textbooks"].as_array().unwrap();
    assert!(!books.is_empty());
    assert!(books.iter().any(|b| b["id"] == "letters"));
    let ch = &books[0]["chapters"][0];
    for key in ["index", "title", "totalCards", "masteredCount", "unlocked", "hint"] {
        assert!(ch.get(key).is_some(), "missing chapter field {key}");
    }
    std::fs::remove_dir_all(&s.data_dir).ok();
}

#[test]
fn dict_search_degrades_gracefully() {
    let s = fresh_state();
    let v: serde_json::Value =
        serde_json::from_str(&commands::dict_search(&s.core, "λογος").unwrap()).unwrap();
    assert!(v.is_array(), "must be an array even without morphology");
    std::fs::remove_dir_all(&s.data_dir).ok();
}

#[test]
fn sync_status_and_nostr_identity_roundtrip() {
    let s = fresh_state();
    let v: serde_json::Value =
        serde_json::from_str(&commands::sync_status(&s).unwrap()).unwrap();
    assert!(v["deviceId"].as_str().is_some_and(|d| !d.is_empty()));
    assert_eq!(v["irohConfigured"], false);
    assert_eq!(v["nostrConfigured"], false);
    assert!(v["relays"].as_str().unwrap().contains("wss://"));
    assert_eq!(v["paired"].as_array().unwrap().len(), 0);

    // Identity ensure: generates once, then returns the SAME key (this
    // exercises the read-only pubkey derivation path for the stored nsec).
    let pk1: String =
        serde_json::from_str(&commands::nostr_identity_ensure(&s).unwrap()).unwrap();
    assert_eq!(pk1.len(), 64, "pubkey hex");
    let pk2: String =
        serde_json::from_str(&commands::nostr_identity_ensure(&s).unwrap()).unwrap();
    assert_eq!(pk1, pk2);

    let v2: serde_json::Value =
        serde_json::from_str(&commands::sync_status(&s).unwrap()).unwrap();
    assert_eq!(v2["nostrConfigured"], true);

    // Relay set roundtrip via sync_status.
    commands::nostr_relays_set(&s.core, "wss://a.example,wss://b.example").unwrap();
    let v3: serde_json::Value =
        serde_json::from_str(&commands::sync_status(&s).unwrap()).unwrap();
    assert_eq!(v3["relays"], "wss://a.example,wss://b.example");
    std::fs::remove_dir_all(&s.data_dir).ok();
}

#[test]
fn backup_export_import_roundtrip() {
    let s = fresh_state();
    // Rate one card so the export has at least one review event.
    commands::review_current(&s).unwrap();
    commands::review_rate(&s, 4).unwrap();

    let exported = commands::backup_export(&s.core).unwrap();
    let batch: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(batch["v"], 1, "raw SyncBatch JSON (not double-encoded)");

    // Importing our own backup: events are uuid-deduped, so 0 applied.
    let res: serde_json::Value =
        serde_json::from_str(&commands::backup_import(&s.core, &exported).unwrap()).unwrap();
    assert_eq!(res["reviewsReceived"], 0);

    // A fresh device importing the same backup DOES receive the review.
    let s2 = fresh_state();
    let res2: serde_json::Value =
        serde_json::from_str(&commands::backup_import(&s2.core, &exported).unwrap()).unwrap();
    assert!(res2["reviewsReceived"].as_u64().unwrap() >= 1);
    std::fs::remove_dir_all(&s.data_dir).ok();
    std::fs::remove_dir_all(&s2.data_dir).ok();
}

#[tokio::test]
async fn sync_now_requires_a_transport() {
    let s = fresh_state();
    let err = commands::sync_now(&s).await.unwrap_err();
    assert!(err.contains("no sync transport configured"), "{err}");
    std::fs::remove_dir_all(&s.data_dir).ok();
}
