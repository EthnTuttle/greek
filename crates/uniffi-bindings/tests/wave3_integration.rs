//! End-to-end integration test for the wave-3 UniFFI surface (plain Rust
//! calls against `GreekCore`, no JNI): generated practice MCQs, typed-answer
//! grading, practice result submission, and practice accuracy stats.

use std::env;
use uniffi_bindings::GreekCore;

fn temp_db_path(tag: &str) -> String {
    let dir = env::temp_dir().join(format!("greek-wave3-{}-{}", tag, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("test.db").to_string_lossy().into_owned()
}

/// `n` cards sharing pos and chapter (and the athenaze tag), so the MCQ
/// generator's preferred distractor policy always has enough material.
fn cards_json(n: usize) -> String {
    let cards: Vec<serde_json::Value> = (0..n)
        .map(|i| {
            serde_json::json!({
                "id": format!("test-card-{}", i),
                "front": format!("λόγος-{}", i),
                "back": format!("word-{}", i),
                "chapter": 1,
                "card_type": "vocab",
                "pos": "noun",
                "tags": ["athenaze"]
            })
        })
        .collect();
    serde_json::to_string(&cards).unwrap()
}

/// Extract the numeric suffix from a "test-card-<i>" id.
fn card_index_of(card_id: &str) -> usize {
    card_id
        .strip_prefix("test-card-")
        .and_then(|s| s.parse().ok())
        .expect("card id has a numeric suffix")
}

#[test]
fn wave3_practice_surface_end_to_end() {
    // 12 cards sharing pos/chapter: generate_practice_mcq(10) must succeed.
    let core = open_core("e2e", &cards_json(12));

    // No practice rows yet.
    let fresh = core.get_practice_stats();
    assert_eq!((fresh.total, fresh.correct), (0, 0));

    // MCQ generation: structural assertions over every question.
    let mcqs = core.generate_practice_mcq(10);
    assert_eq!(mcqs.len(), 10, "10 of 12 cards should yield questions");
    let mut seen_ids = std::collections::HashSet::new();
    for mcq in &mcqs {
        assert_eq!(mcq.choices.len(), 4, "choices: {:?}", mcq.choices);
        assert!((mcq.correct_index as usize) < mcq.choices.len());
        // The prompt is that card's English back; the choice at
        // correct_index is that card's Greek front.
        let i = card_index_of(&mcq.card_id);
        assert_eq!(mcq.prompt_gloss, format!("word-{i}"));
        assert_eq!(
            mcq.choices[mcq.correct_index as usize],
            format!("λόγος-{i}"),
            "correct choice must be the card's own front"
        );
        let mut uniq = mcq.choices.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), 4, "choices unique: {:?}", mcq.choices);
        assert!(seen_ids.insert(mcq.card_id.clone()), "no repeated prompts");
    }

    // Typed answers against a known card: accent-free input is accepted
    // leniently, only the exact spelling counts as Exact.
    assert!(core.check_typed_answer("test-card-0".to_string(), "λογος-0".to_string()));
    assert!(!core.typed_answer_exact("test-card-0".to_string(), "λογος-0".to_string()));
    assert!(core.typed_answer_exact("test-card-0".to_string(), "λόγος-0".to_string()));
    assert!(core.check_typed_answer("test-card-0".to_string(), "  λόγος-0 ".to_string()));
    // Wrong answer and unknown card both fail both checks.
    assert!(!core.check_typed_answer("test-card-0".to_string(), "λόγος-1".to_string()));
    assert!(!core.check_typed_answer("nope".to_string(), "λόγος-0".to_string()));
    assert!(!core.typed_answer_exact("nope".to_string(), "λόγος-0".to_string()));

    // Practice results are recorded and surface in the accuracy stats:
    // one exact-correct (rating 4) and one wrong (rating 1).
    assert!(core.submit_practice_result("test-card-0".to_string(), true, true));
    assert!(core.submit_practice_result("test-card-1".to_string(), false, false));
    let stats = core.get_practice_stats();
    assert_eq!((stats.total, stats.correct), (2, 1));

    // Practice answers are tagged 'practice' in review_logs, never 'flashcard'.
    // (The plain flashcard flow still records its own source separately.)
    assert!(core.submit_review("test-card-2".to_string(), 3));
    let stats_after_flashcard = core.get_practice_stats();
    assert_eq!(
        (stats_after_flashcard.total, stats_after_flashcard.correct),
        (2, 1),
        "flashcard reviews must not leak into practice stats"
    );
}

/// A fresh core over a temp database with the given inline cards JSON.
fn open_core(tag: &str, cards: &str) -> std::sync::Arc<GreekCore> {
    GreekCore::new_with_cards(temp_db_path(tag), None, cards.to_string()).unwrap()
}
