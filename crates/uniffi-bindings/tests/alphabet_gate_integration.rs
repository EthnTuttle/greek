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
    let db_path = temp_db_path("e2e");
    let core = GreekCore::new_with_cards(db_path.clone(), None, gate_cards_json()).unwrap();

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

    // unlock_chapter refuses both an alphabet-gated vocabulary chapter and
    // a letters chapter whose own prerequisite mastery is still unmet (C11).
    assert!(!core.unlock_chapter("athenaze".to_string(), 2));
    assert!(!core.unlock_chapter("letters".to_string(), 2));
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    for n in 1..=6 {
        conn.execute(
            "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
             VALUES (?1, 30.0, 5.0, '2020-01-01T00:00:00Z', 2, 0)",
            [format!("letters-{n:03}")],
        )
        .unwrap();
    }
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
        .starts_with("Learn 80% of Chapter 1"));

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

    // After the pass: ch.1 introduction flows, but ch.2 still requires its
    // own prerequisite mastery. The gate is not a progression bypass (C11).
    assert!(!core.get_new_cards("athenaze".to_string(), 1, 10).is_empty());
    assert!(!core.unlock_chapter("athenaze".to_string(), 2));
    for n in 1..=5 {
        let id = format!("ath1-{n:03}");
        conn.execute(
            "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
             VALUES (?1, 30.0, 5.0, '2020-01-01T00:00:00Z', 5, 0)",
            [&id],
        )
        .unwrap();
        for day in ["2020-01-01", "2020-01-02"] {
            conn.execute(
                "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                 VALUES (?1, ?2, 3, 0.0, 0.0)",
                rusqlite::params![id, format!("{day}T12:00:00Z")],
            )
            .unwrap();
        }
    }
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
    let db_path = temp_db_path("debt");
    let core = GreekCore::new_with_cards(db_path.clone(), None, gate_cards_json()).unwrap();
    assert!(!core.get_alphabet_gate_state().satisfied);

    // C2: the reader's "Add to SRS" lookup flow now writes NO memory_states
    // row — an unreviewed lookup card is neither due nor vocab history.
    let lookup_id = core
        .add_card_from_lookup("λόγος".to_string(), "".to_string(), "word".to_string())
        .unwrap();
    assert!(
        core.get_due_cards(10)
            .unwrap()
            .iter()
            .all(|c| c.card.id != lookup_id),
        "an unreviewed lookup card must not appear as due"
    );
    assert!(
        !core.get_alphabet_gate_state().satisfied,
        "merely adding a lookup card is not a vocabulary habit"
    );

    // Genuine pre-gate review debt: plant a REAL reviewed vocab row due in
    // the past (what a pre-gate install's database actually carries). The
    // public API can't backdate a due date, so plant it on a second
    // connection (WAL mode allows the concurrent reader/writer pair).
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "INSERT INTO memory_states (card_id, stability, difficulty, last_review, due_date, reps, lapses)
         VALUES ('ath1-001', 3.0, 5.0, '2020-01-01T00:00:00Z', '2020-01-02T00:00:00Z', 1, 0)",
        [],
    )
    .unwrap();

    let due = core.get_due_cards(10).unwrap();
    let debt = due
        .iter()
        .find(|c| c.card.id == "ath1-001")
        .expect("due review debt must never be gated");
    assert!(debt.card.front.contains("λόγος"), "deck content flows");
    let state = core.get_alphabet_gate_state();
    assert!(state.satisfied);
    assert_eq!(state.method, "vocab_history");
}
