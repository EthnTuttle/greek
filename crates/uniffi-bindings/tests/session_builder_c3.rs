//! Regression tests for C3: building a review session must consume ONE
//! daily new-card budget snapshot, not one per chapter. The old Android
//! loop called `get_new_cards` per chapter; each call re-saw the full
//! unspent allowance, so a 10/day cap admitted a 7+10+3 multi-curriculum
//! session. `build_review_session` admits at most the day's true remainder
//! across ALL curricula and chapters.

use std::env;
use uniffi_bindings::GreekCore;

fn temp_db_path(tag: &str) -> String {
    let dir = env::temp_dir().join(format!(
        "greek-session-builder-{}-{}",
        tag,
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("test.db").to_string_lossy().into_owned()
}

/// The real letters track (ch.1 has exactly 7 cards — the repro's "7
/// Letters") plus Athenaze ch.1 (12 cards) and JACT ch.1 (5 cards): enough
/// supply in EVERY track that the per-chapter loop could over-admit.
fn repro_deck_json() -> String {
    let mut cards: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../data/cards/letters.json"))
            .expect("letters.json must parse");
    let vocab = |id: &str, chapter: i32, tag: &str| {
        serde_json::json!({
            "id": id,
            "front": format!("λόγος-{id}"),
            "back": format!("word {id}"),
            "chapter": chapter,
            "card_type": "Vocab",
            "pos": "noun",
            "tags": [tag, format!("chapter-{chapter}")]
        })
    };
    for i in 1..=12 {
        cards.push(vocab(&format!("ath1-{i:03}"), 1, "athenaze-i"));
    }
    for i in 1..=5 {
        cards.push(vocab(&format!("jact1-{i:03}"), 1, "jact"));
    }
    serde_json::to_string(&cards).unwrap()
}

#[test]
fn session_builder_single_budget_across_curricula() {
    let core =
        GreekCore::new_with_cards(temp_db_path("budget"), None, repro_deck_json()).unwrap();
    assert!(core.declare_alphabet_known());
    let remaining = core.new_cards_remaining_today();
    assert_eq!(remaining, 10, "fresh day, full 10/day cap (DEFAULT_DAILY_NEW_CAP)");

    let session = core.build_review_session(20).unwrap();

    // The gate is satisfied and no reviews exist, so the whole session is new
    // cards, and its new-card count is EXACTLY the remaining budget — not
    // one budget per chapter.
    let new_count = session.iter().filter(|c| c.is_new).count() as u32;
    assert_eq!(new_count, remaining);
    assert!(session.len() <= 20);

    // Filling order preserved: letters ch.1 (7 cards) first, then Athenaze
    // ch.1 for the leftover allowance (3), JACT cut off by the spent budget.
    let letters = session.iter().filter(|c| c.card.id.starts_with("letters-")).count();
    let ath = session.iter().filter(|c| c.card.id.starts_with("ath1-")).count();
    let jact = session.iter().filter(|c| c.card.id.starts_with("jact1-")).count();
    assert_eq!((letters, ath, jact), (7, 3, 0), "7+3, not the repro's 7+10+3");
    assert!(session[0].card.id.starts_with("letters-"));

    // The per-chapter path still caps per call; the SESSION builder is the
    // only immune one. (Deprecated for this use — see lib.rs.)
    assert_eq!(core.get_new_cards("athenaze".to_string(), 1, 20).len(), 10);

    // Submitting the entire built queue can never beat the daily cap.
    for sc in &session {
        assert!(core.submit_review(sc.card.id.clone(), 3));
    }
    assert!(core.new_cards_today() <= 10);
    assert_eq!(core.new_cards_today(), remaining);

    // Budget spent: a second session this same day introduces nothing new.
    let session2 = core.build_review_session(20).unwrap();
    assert!(session2.iter().all(|c| !c.is_new));
}

#[test]
fn session_builder_due_first_then_new() {
    let db_path = temp_db_path("order");
    let core = GreekCore::new_with_cards(db_path.clone(), None, repro_deck_json()).unwrap();
    assert!(core.declare_alphabet_known());

    // Plant two overdue vocab reviews (memory_states + a YESTERDAY review
    // log — first logs dated today would count as new-today) on a second
    // connection; WAL allows the concurrent reader/writer pair (pattern from
    // alphabet_gate_integration.rs).
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    for (id, due, ts) in [
        ("ath1-001", "2020-01-02T00:00:00Z", "2020-01-01T00:00:00Z"),
        ("ath1-002", "2020-01-03T00:00:00Z", "2020-01-01T00:00:00Z"),
    ] {
        conn.execute(
            "INSERT INTO memory_states (card_id, stability, difficulty, last_review, due_date, reps, lapses)
             VALUES (?1, 3.0, 5.0, ?3, ?2, 1, 0)",
            rusqlite::params![id, due, ts],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
             VALUES (?1, ?2, 3, 0.0, 0.0)",
            rusqlite::params![id, ts],
        )
        .unwrap();
    }

    let session = core.build_review_session(20).unwrap();

    // Due cards lead the session, most overdue first, with deck content.
    assert_eq!(session[0].card.id, "ath1-001");
    assert_eq!(session[1].card.id, "ath1-002");
    assert!(!session[0].is_new && !session[1].is_new);
    assert!(session[0].card.front.contains("λόγος"));
    // No due card appears past the first new card.
    let first_new = session.iter().position(|c| c.is_new).expect("new cards follow");
    assert_eq!(first_new, 2);

    // Due reviews never consume the new-card budget: still the full 10 new
    // (7 letters + 3 Athenaze; the two planted ath1 cards are excluded as
    // already-known), 12 cards total.
    assert_eq!(session.iter().filter(|c| c.is_new).count(), 10);
    assert_eq!(session.len(), 12);

    // Full submission of the built queue stays within the day's cap.
    for sc in &session {
        assert!(core.submit_review(sc.card.id.clone(), 3));
    }
    assert!(core.new_cards_today() <= 10);
    assert_eq!(core.new_cards_today(), 10);
}

#[test]
fn session_builder_respects_alphabet_gate() {
    // Gate UNMET: vocabulary tracks contribute no new cards; letters flow.
    let core =
        GreekCore::new_with_cards(temp_db_path("gate"), None, repro_deck_json()).unwrap();
    assert!(!core.get_alphabet_gate_state().satisfied);

    let session = core.build_review_session(20).unwrap();
    assert!(!session.is_empty());
    assert!(session.iter().all(|c| c.card.id.starts_with("letters-")));
    assert_eq!(session.len(), 7, "only letters ch.1 is unlocked by default");
}
