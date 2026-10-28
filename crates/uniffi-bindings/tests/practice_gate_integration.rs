//! Integration tests for the practice/gate policy (C4/C5):
//! `generate_practice_mcq` draws only from gate-eligible cards (letters
//! always; vocabulary once the gate latches), and `submit_practice_result`
//! refuses to create practice state for ineligible cards so it can never
//! fabricate "vocab history" and falsely satisfy the alphabet gate.

use std::env;
use uniffi_bindings::GreekCore;

fn temp_db_path(tag: &str) -> String {
    let dir = env::temp_dir().join(format!(
        "greek-practice-gate-{}-{}",
        tag,
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("test.db").to_string_lossy().into_owned()
}

/// Same shape as `alphabet_gate_integration.rs`: the real letters track
/// plus a small Athenaze ch.1/2 slice, as one JSON array.
fn gate_cards_json() -> String {
    let mut cards: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../data/cards/letters.json"))
            .expect("letters.json must parse");
    let ath = |id: &str, chapter: i32| {
        serde_json::json!({
            "id": id,
            "front": format!("ὁ λόγος-{id}"),
            "back": format!("the word-{id}"),
            "chapter": chapter,
            "card_type": "Vocab",
            "pos": "noun",
            "tags": ["athenaze-i", format!("chapter-{chapter}")]
        })
    };
    for i in 1..=6 {
        cards.push(ath(&format!("ath1-{i:03}"), 1));
    }
    for i in 1..=4 {
        cards.push(ath(&format!("ath2-{i:03}"), 2));
    }
    serde_json::to_string(&cards).unwrap()
}

/// A fresh core over a temp database with the gate fixture.
fn open_core(tag: &str) -> std::sync::Arc<GreekCore> {
    GreekCore::new_with_cards(temp_db_path(tag), None, gate_cards_json()).unwrap()
}

#[test]
fn practice_draw_pool_is_gated() {
    let core = open_core("pool");

    // Fresh core (gate unsatisfied): practice offers letters only.
    assert!(!core.get_alphabet_gate_state().satisfied);
    let mcqs = core.generate_practice_mcq(10);
    assert!(
        !mcqs.is_empty(),
        "letters practice must flow on a fresh core"
    );
    assert!(
        mcqs.iter().all(|m| m.card_id.starts_with("letters-")),
        "gate unsatisfied: practice must be letters-only, got {:?}",
        mcqs.iter().map(|m| m.card_id.clone()).collect::<Vec<_>>()
    );

    // After latching the gate, vocabulary joins the practice pool and is
    // accepted for submission.
    assert!(core.declare_alphabet_known());
    // Reviewed-first tiering: give one vocab card a state, then the
    // reviewed tier (deck-ordered) still samples from the full tier (C5);
    // assert only that generation succeeds and remains structurally sound.
    assert!(core.submit_practice_result("ath1-001".to_string(), true, false));
    let mcqs = core.generate_practice_mcq(10);
    assert!(!mcqs.is_empty());
    assert!(mcqs.iter().all(|m| {
        m.card_id.starts_with("letters-")
            || m.card_id.starts_with("ath1-")
            || m.card_id.starts_with("ath2-")
    }));
}

#[test]
fn practice_answer_cannot_fabricate_vocab_history() {
    let core = open_core("belt");

    // A practice-typed vocabulary answer on a fresh core (gate unsatisfied)
    // is refused and records NOTHING — no memory state, no review log.
    assert!(!core.get_alphabet_gate_state().satisfied);
    assert!(
        !core.submit_practice_result("ath1-001".to_string(), true, false),
        "ineligible practice submissions must be refused"
    );
    let stats = core.get_practice_stats();
    assert_eq!(
        (stats.total, stats.correct),
        (0, 0),
        "a refused submission must not count in practice stats"
    );
    // ...and the gate stays unsatisfied (the Lane-C vocab-history
    // grandfather never sees the fabricated row).
    let state = core.get_alphabet_gate_state();
    assert!(
        !state.satisfied,
        "one practice vocab answer must NOT complete the alphabet"
    );

    // Letters practice still records normally (it feeds the organic lane).
    let mcqs = core.generate_practice_mcq(4);
    let letter = mcqs
        .first()
        .expect("letters practice available")
        .card_id
        .clone();
    assert!(letter.starts_with("letters-"));
    assert!(core.submit_practice_result(letter, true, true));
    let stats = core.get_practice_stats();
    assert_eq!((stats.total, stats.correct), (1, 1));

    // After the latch, vocabulary practice submissions are accepted.
    assert!(core.declare_alphabet_known());
    assert!(core.submit_practice_result("ath1-002".to_string(), false, false));
    let stats = core.get_practice_stats();
    assert_eq!((stats.total, stats.correct), (2, 1));
}
