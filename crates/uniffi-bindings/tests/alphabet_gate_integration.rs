//! End-to-end integration test for the alphabet gate (DESIGN-alphabet-gate.md
//! §8): vocabulary new-card introduction is blocked until the gate is met;
//! due reviews always flow; passing the Alphabet Check unlocks vocabulary.

use std::env;
use uniffi_bindings::GreekCore;

fn temp_db_path(tag: &str) -> String {
    let dir = env::temp_dir().join(format!("greek-alphabet-gate-{}-{}", tag, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("test.db").to_string_lossy().into_owned()
}

/// The real letters track plus a small Athenaze ch.1/2 slice, as one JSON
/// array (the same JSON the app passes to `new_with_cards`).
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
fn alphabet_gate_end_to_end() {
    let core = open_core("e2e");

    // Fresh install: the gate is unmet, counting 43 loaded letters cards.
    let state = core.get_alphabet_gate_state();
    assert!(!state.satisfied);
    assert_eq!(state.method, "");
    assert_eq!(state.letters_total, 43);
    assert_eq!(state.letters_introduced, 0);

    // While unmet: vocabulary new-card introduction is refused, letters flow.
    assert!(core.get_new_cards("athenaze".to_string(), 1, 10).is_empty());
    assert!(core.get_new_cards("dcc".to_string(), 0, 10).is_empty());
    assert!(!core.get_new_cards("letters".to_string(), 1, 10).is_empty());

    // unlock_chapter is refused for vocabulary tracks, allowed for letters.
    assert!(!core.unlock_chapter("athenaze".to_string(), 2));
    assert!(core.unlock_chapter("letters".to_string(), 2));

    // Vocab chapters all report locked; the default chapter ch.1 carries the
    // gate hint while later chapters keep their prev-chapter hints.
    let chapters = core.get_chapters("athenaze".to_string());
    assert!(chapters.iter().all(|c| !c.unlocked));
    assert!(chapters[0]
        .unlock_hint
        .contains("pass the Alphabet Check"));
    assert!(chapters[1]
        .unlock_hint
        .starts_with("Master 80% of Chapter 1"));

    // The Alphabet Check itself: 24 items, 4 choices, both polarities.
    let check = core.generate_alphabet_check();
    assert!(
        check.len() >= 20,
        "24-question check expected, got {}",
        check.len()
    );
    assert!(check.iter().any(|q| q.glyph_to_name));
    assert!(check.iter().any(|q| !q.glyph_to_name));
    assert!(check
        .iter()
        .all(|q| q.choices.len() == 4 && (q.correct_index as usize) < 4));

    // A failing attempt leaves the gate closed.
    assert!(!core.record_alphabet_check_result(21, 24));
    assert!(!core.get_alphabet_gate_state().satisfied);

    // Simulate the miss-recycling wire-through on the failing attempt, then
    // a passing one (23/24) at the verdict level.
    assert!(core.submit_practice_result(check[0].card_id.clone(), false, false));
    assert!(core.record_alphabet_check_result(23, 24));
    let state = core.get_alphabet_gate_state();
    assert!(state.satisfied);
    assert_eq!(state.method, "check_passed");

    // After the pass: vocabulary introduction and manual unlocks flow.
    assert!(!core.get_new_cards("athenaze".to_string(), 1, 10).is_empty());
    assert!(core.unlock_chapter("athenaze".to_string(), 2));
    let chapters = core.get_chapters("athenaze".to_string());
    assert!(chapters[0].unlocked);
    assert!(chapters[1].unlocked, "the manual unlock must stick");

    // Declared-know latch on a third core (Settings skip).
    let core = open_core("declared");
    assert!(core.declare_alphabet_known());
    let state = core.get_alphabet_gate_state();
    assert!(state.satisfied);
    assert_eq!(state.method, "declared_known");
    assert!(!core.get_new_cards("athenaze".to_string(), 1, 10).is_empty());
}

#[test]
fn alphabet_gate_grandfathers_vocab_history() {
    let core = open_core("grandfathered");
    // One pre-existing vocabulary review (through the normal flashcard
    // flow) means this learner had a pre-gate vocab habit: the gate
    // auto-satisfies without any letters work.
    assert!(core.submit_review("ath1-001".to_string(), 3));
    let state = core.get_alphabet_gate_state();
    assert!(state.satisfied);
    assert_eq!(state.method, "vocab_history");
    assert!(!core.get_new_cards("athenaze".to_string(), 1, 10).is_empty());
}

#[test]
fn due_review_debt_is_never_gated() {
    // get_due_cards takes no part in the gate: a due vocab-side card flows
    // even on a day-one install. (Any due vocab card implies vocab history,
    // which also grandfathers the gate — asserted here to pin both halves
    // of the legacy-user story.)
    let core = open_core("debt");
    assert!(!core.get_alphabet_gate_state().satisfied);
    // The reader's "Add to SRS" lookup flow creates an immediately-due card.
    let lookup_id = core
        .add_card_from_lookup("λόγος".to_string(), "".to_string(), "word".to_string())
        .unwrap();
    let due = core.get_due_cards(10).unwrap();
    assert!(
        due.iter().any(|c| c.card.id == lookup_id),
        "due review debt must never be gated"
    );
    let state = core.get_alphabet_gate_state();
    assert!(state.satisfied);
    assert_eq!(state.method, "vocab_history");
}
