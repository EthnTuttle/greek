//! Owner-approved learned-not-mature pacing, cross-deck dedupe, and reader
//! lookup resurfacing. These tests use the real Greek card data where useful.

use std::env;
use uniffi_bindings::GreekCore;

fn temp_db_path(tag: &str) -> String {
    let dir = env::temp_dir().join(format!("greek-pacing-{}-{}", tag, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("test.db").to_string_lossy().into_owned()
}

fn combined_cards(include_letters: bool) -> String {
    let mut cards: Vec<serde_json::Value> = if include_letters {
        serde_json::from_str(include_str!("../../../data/cards/letters.json")).unwrap()
    } else {
        Vec::new()
    };
    for deck in [
        include_str!("../../../data/cards/athenaze-i.json"),
        include_str!("../../../data/cards/dcc-core.json"),
    ] {
        cards.extend(serde_json::from_str::<Vec<serde_json::Value>>(deck).unwrap());
    }
    serde_json::to_string(&cards).unwrap()
}

#[test]
fn twenty_three_card_chapter_learned_on_two_days_auto_unlocks_and_admits_next() {
    let db_path = temp_db_path("chapter-23");
    let cards = include_str!("../../../data/cards/athenaze-i.json").to_string();
    let core = GreekCore::new_with_cards(db_path.clone(), None, cards.clone()).unwrap();
    let first: Vec<serde_json::Value> = serde_json::from_str::<Vec<serde_json::Value>>(&cards)
        .unwrap()
        .into_iter()
        .filter(|card| card["chapter"] == 1)
        .collect();
    assert_eq!(first.len(), 23);

    // Day 1 introduces all 23; day 2's Good is the most recent rating.
    // Stability remains only one day, proving that 30d mastery is not a gate.
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    for card in first {
        let id = card["id"].as_str().unwrap();
        conn.execute(
            "INSERT INTO memory_states
                 (card_id, stability, difficulty, last_review, due_date, reps, lapses)
             VALUES (?1, 1.0, 5.0, '2024-01-02T12:00:00Z',
                     '2024-01-03T12:00:00Z', 2, 0)",
            [id],
        )
        .unwrap();
        for day in ["2024-01-01", "2024-01-02"] {
            conn.execute(
                "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                 VALUES (?1, ?2, 3, 0.0, 0.0)",
                rusqlite::params![id, format!("{day}T12:00:00Z")],
            )
            .unwrap();
        }
    }

    let chapters = core.get_chapters("athenaze".to_string());
    assert_eq!(chapters[0].learning_count, 23);
    assert_eq!(chapters[0].mastered_count, 0);
    assert!(chapters[1].unlocked, "chapter 2 must auto-unlock");
    assert_eq!(
        conn.query_row(
            "SELECT unlocked FROM curriculum_progress
             WHERE textbook = 'athenaze' AND chapter = 2",
            [],
            |row| row.get::<_, i32>(0),
        )
        .unwrap(),
        1,
        "the admission decision must be durably latched"
    );

    let session = core.build_review_session(100).unwrap();
    assert!(
        session
            .iter()
            .any(|card| card.is_new && card.card.chapter == 2),
        "new chapter cards must enter the unified session"
    );
    let stats = core.get_review_stats();
    assert_eq!(stats.learning_count, 23);
    assert_eq!(stats.mastered_count, 0);
}

#[test]
fn easy_first_exposure_cannot_stall_letters_chapter_two() {
    let core = GreekCore::new_with_cards(
        temp_db_path("easy-letters"),
        None,
        include_str!("../../../data/cards/letters.json").to_string(),
    )
    .unwrap();
    for n in 1..=6 {
        assert!(core.submit_review(format!("letters-{n:03}"), 4));
    }
    let chapters = core.get_chapters("letters".to_string());
    assert_eq!(chapters[0].learning_count, 6);
    assert_eq!(chapters[0].mastered_count, 0);
    assert!(chapters[1].unlocked);

    let session = core.build_review_session(100).unwrap();
    assert!(
        session
            .iter()
            .any(|card| card.is_new && card.card.chapter == 2),
        "the second letters chapter must admit new cards immediately"
    );
}

#[test]
fn normalized_calos_dedupes_before_spending_session_capacity() {
    let mut cards = vec![serde_json::json!({
        "id": "ath1-calos",
        "front": "καλός, -ή, -όν",
        "back": "beautiful, good",
        "chapter": 1,
        "card_type": "Vocab",
        "pos": "adjective",
        "tags": ["athenaze-i"]
    })];
    // Enough DISTINCT cards for all ten slots; the DCC twin must not reduce
    // the admitted unique count to nine.
    for n in 1..=10 {
        cards.push(serde_json::json!({
            "id": format!("dcc-{n:03}"),
            "front": format!("λέξις-{n}"),
            "back": "word",
            "chapter": 0,
            "card_type": "Vocab",
            "pos": "noun",
            "tags": ["dcc-core"]
        }));
    }
    cards.push(serde_json::json!({
        "id": "dcc-091",
        "front": "καλόσ",
        "back": "beautiful, good",
        "chapter": 0,
        "card_type": "Vocab",
        "pos": "adjective",
        "tags": ["dcc-core"]
    }));
    let core = GreekCore::new_with_cards(
        temp_db_path("dedupe-calos"),
        None,
        serde_json::to_string(&cards).unwrap(),
    )
    .unwrap();
    let session = core.build_review_session(20).unwrap();
    assert_eq!(session.iter().filter(|card| card.is_new).count(), 10);
    assert!(session.iter().any(|card| card.card.id == "ath1-calos"));
    assert!(!session.iter().any(|card| card.card.id == "dcc-091"));

    // A state under the first ID also excludes the duplicate on a later build,
    // independently of same-session seen-key suppression.
    for card in session {
        assert!(core.submit_review(card.card.id, 3));
    }
    assert!(!core
        .get_new_cards("dcc".to_string(), 0, 20)
        .iter()
        .any(|card| card.id == "dcc-091"));
}

#[test]
fn real_greek_lookup_resurfaces_then_leaves_the_new_pool() {
    let core = GreekCore::new_with_cards(
        temp_db_path("lookup-logos"),
        None,
        include_str!("../../../data/cards/athenaze-i.json").to_string(),
    )
    .unwrap();
    core.mark_word_known("λόγος".to_string(), true).unwrap();
    assert_eq!(core.get_review_stats().learning_count, 0);

    let id = core
        .add_card_from_lookup_with_context(
            "λόγος".to_string(),
            "λόγου".to_string(),
            "word; reason".to_string(),
            "ἀπὸ τοῦ λόγου".to_string(),
        )
        .unwrap();
    let session = core.build_review_session(20).unwrap();
    let lookup = session.iter().find(|card| card.card.id == id).unwrap();
    assert!(lookup.is_new);
    assert_eq!(session.first().unwrap().card.id, id);
    assert!(lookup.card.front.contains("λόγου"));
    assert!(lookup.card.front.contains("ἀπὸ τοῦ λόγου"));
    assert!(lookup.card.back.contains("λόγος"));
    assert!(lookup.card.back.contains("word; reason"));

    assert!(core.submit_review(id.clone(), 3));
    let later = core.build_review_session(20).unwrap();
    assert!(!later.iter().any(|card| card.is_new && card.card.id == id));
    let stats = core.get_review_stats();
    assert_eq!(stats.learning_count, 1);
    assert_eq!(stats.mastered_count, 0);
}

#[test]
fn principal_parts_are_not_suppressed_by_existing_base_verb_state() {
    let db_path = temp_db_path("dedupe-principal-parts");
    let cards = serde_json::json!([
        {
            "id": "ath1-phero",
            "front": "φέρω",
            "back": "carry",
            "chapter": 1,
            "card_type": "Vocab",
            "pos": "verb",
            "tags": ["athenaze-i"]
        },
        {
            "id": "dcc-pp-phero",
            "front": "φέρω (principal parts)",
            "back": "carry: φέρω, οἴσω, …",
            "chapter": 0,
            "card_type": "Vocab",
            "pos": "verb",
            "card_type": "PrincipalParts",
            "tags": ["dcc-core", "principal-parts"]
        },
        {
            "id": "dcc-other",
            "front": "λόγος",
            "back": "word",
            "chapter": 0,
            "card_type": "Vocab",
            "pos": "noun",
            "tags": ["dcc-core"]
        }
    ]);
    let core = GreekCore::new_with_cards(
        db_path.clone(),
        None,
        serde_json::to_string(&cards).unwrap(),
    )
    .unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "INSERT INTO memory_states
             (card_id, stability, difficulty, last_review, due_date, reps, lapses)
         VALUES ('ath1-phero', 2.0, 5.0, '2020-01-01T00:00:00Z',
                 '2020-01-02T00:00:00Z', 1, 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
         VALUES ('ath1-phero', '2020-01-01T00:00:00Z', 3, 0.0, 0.0)",
        [],
    )
    .unwrap();

    let session = core.build_review_session(20).unwrap();
    assert!(session.iter().any(|card| card.card.id == "dcc-other"));
    assert!(session.iter().any(|card| card.card.id == "dcc-pp-phero"));
    assert_eq!(core.new_cards_remaining_today(), 10);
}

fn all_shipped_cards() -> String {
    let mut cards = Vec::new();
    for deck in [
        include_str!("../../../data/cards/letters.json"),
        include_str!("../../../data/cards/athenaze-i.json"),
        include_str!("../../../data/cards/athenaze-ii.json"),
        include_str!("../../../data/cards/jact.json"),
        include_str!("../../../data/cards/dcc-core.json"),
    ] {
        cards.extend(serde_json::from_str::<Vec<serde_json::Value>>(deck).unwrap());
    }
    assert_eq!(cards.len(), 1183);
    serde_json::to_string(&cards).unwrap()
}

fn seed_old_learned(conn: &rusqlite::Connection, id: &str) {
    conn.execute(
        "INSERT INTO memory_states
             (card_id, stability, difficulty, last_review, due_date, reps, lapses)
         VALUES (?1, 1.0, 5.0, '2024-01-02T12:00:00Z',
                 '2099-01-03T12:00:00Z', 2, 0)",
        [id],
    )
    .unwrap();
    for day in ["2024-01-01", "2024-01-02"] {
        conn.execute(
            "INSERT INTO review_logs
                 (card_id, timestamp, rating, elapsed_days, scheduled_days)
             VALUES (?1, ?2, 3, 0.0, 0.0)",
            rusqlite::params![id, format!("{day}T12:00:00Z")],
        )
        .unwrap();
    }
}

/// Repeatedly build the REAL session, seed its admitted NEW cards as learned
/// on two old dates (so the replay never hits today's cap and never becomes
/// due), and ask every curriculum to latch newly satisfied chapters.
fn replay_new_cards_until_exhausted(core: &GreekCore, db_path: &str) -> (usize, usize) {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    let mut batches = 0;
    let mut exhausted = false;
    for _ in 0..200 {
        let new_ids: Vec<String> = core
            .build_review_session(2000)
            .unwrap()
            .into_iter()
            .filter(|card| card.is_new)
            .map(|card| card.card.id)
            .collect();
        if new_ids.is_empty() {
            exhausted = true;
            break;
        }
        batches += 1;
        for id in new_ids {
            seed_old_learned(&conn, &id);
        }
        for textbook in core.get_curricula() {
            let _ = core.get_chapters(textbook.textbook);
        }
    }
    assert!(
        exhausted,
        "shipped-deck replay deadlocked after {batches} batches"
    );
    let states = conn
        .query_row("SELECT COUNT(*) FROM memory_states", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap();
    (states, batches)
}

fn learn_currently_admissible_chapter(
    core: &GreekCore,
    conn: &rusqlite::Connection,
    textbook: &str,
    chapter: i32,
) -> usize {
    let mut learned = 0;
    for _ in 0..100 {
        let ids: Vec<String> = core
            .get_new_cards(textbook.to_string(), chapter, 1000)
            .into_iter()
            .map(|card| card.id)
            .collect();
        if ids.is_empty() {
            return learned;
        }
        learned += ids.len();
        for id in ids {
            seed_old_learned(conn, &id);
        }
    }
    panic!("{textbook} ch{chapter} did not exhaust its admissible cards");
}

#[test]
fn fable_jact_schedule_has_ten_admissible_of_thirteen_and_unlocks_chapter_two() {
    let db_path = temp_db_path("jact-original-deadlock");
    let core = GreekCore::new_with_cards(db_path.clone(), None, all_shipped_cards()).unwrap();
    assert!(core.declare_alphabet_known());
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    assert_eq!(
        learn_currently_admissible_chapter(&core, &conn, "athenaze", 1),
        23
    );

    let introduced = learn_currently_admissible_chapter(&core, &conn, "jact", 1);
    assert_eq!(introduced, 10, "three exact headword twins must dedupe");
    let chapters = core.get_chapters("jact".to_string());
    assert_eq!(chapters[0].learning_count, 10);
    assert!(
        chapters[1].unlocked,
        "10/13 must use denominator ten (need eight), not the impossible eleven of thirteen"
    );
}

#[test]
fn shipped_decks_walk_every_populated_chapter_without_dedupe_deadlock() {
    let db_path = temp_db_path("all-shipped-replay");
    let cards = all_shipped_cards();
    let core = GreekCore::new_with_cards(db_path.clone(), None, cards).unwrap();
    assert!(core.declare_alphabet_known());
    let (states, batches) = replay_new_cards_until_exhausted(&core, &db_path);
    assert!(states > 600, "regression reached only {states}/1183 units");

    for textbook in core.get_curricula() {
        for chapter in core
            .get_chapters(textbook.textbook.clone())
            .into_iter()
            .filter(|chapter| chapter.available)
        {
            assert!(
                chapter.unlocked,
                "{} chapter {} remained locked after its admissible units were learned",
                textbook.textbook, chapter.chapter
            );
        }
    }

    // The formerly impossible next chapter is open even in this adversarial
    // "all Athenaze first" schedule, where more JACT twins are suppressed.
    let jact = core.get_chapters("jact".to_string());
    assert_eq!(jact[0].card_count, 13);
    assert!(jact[0].learning_count <= 10);
    assert!(jact[1].unlocked);
    assert!(core.get_chapters("athenaze".to_string())[2].unlocked);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let shipped: Vec<greek_core::Card> = serde_json::from_str(&all_shipped_cards()).unwrap();
    for headword in ["εἰς", "εἷς", "τις", "τίς", "εἰμί", "εἶμι"] {
        let key = greek_core::curriculum::normalized_headword_preserving_diacritics(headword);
        let represented = shipped.iter().any(|card| {
            !greek_core::curriculum::is_principal_parts(card)
                && greek_core::curriculum::learning_unit_key(card).as_deref() == Some(key.as_str())
                && conn
                    .query_row(
                        "SELECT COUNT(*) FROM memory_states WHERE card_id = ?1",
                        [&card.id],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap()
                    == 1
        });
        assert!(represented, "meaning-distinct key {key} was never admitted");
    }
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_states WHERE card_id = 'dcc-pp-020'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1,
        "principal parts must survive their plain-verb state"
    );
    for n in 26..=32 {
        assert!(
            conn.query_row(
                "SELECT COUNT(*) FROM memory_states WHERE card_id = ?1",
                [format!("letters-{n:03}")],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                == 1,
            "meaning-distinct breathing/accent lesson {n} was collapsed"
        );
    }
    eprintln!(
        "shipped replay: {states}/1183 introduced over {batches} capped sessions; all populated chapters unlocked"
    );
}

#[test]
fn all_43_shipped_letters_can_unlock_chapter_five_and_organic_lane() {
    let db_path = temp_db_path("organic-43-replay");
    let core = GreekCore::new_with_cards(
        db_path.clone(),
        None,
        include_str!("../../../data/cards/letters.json").to_string(),
    )
    .unwrap();
    let (states, _) = replay_new_cards_until_exhausted(&core, &db_path);
    assert_eq!(states, 43);
    assert!(core
        .get_chapters("letters".to_string())
        .iter()
        .all(|chapter| chapter.unlocked));

    // Complete the remaining organic-lane requirements with measured hits
    // across the same two days used by the replay.
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    for n in 0..20 {
        conn.execute(
            "INSERT INTO review_logs
                 (card_id, timestamp, rating, elapsed_days, scheduled_days, source)
             VALUES (?1, ?2, 3, 0.0, 0.0, 'practice')",
            rusqlite::params![
                format!("letters-{:03}", (n % 43) + 1),
                if n % 2 == 0 {
                    "2024-01-01T13:00:00Z"
                } else {
                    "2024-01-02T13:00:00Z"
                }
            ],
        )
        .unwrap();
    }
    let gate = core.get_alphabet_gate_state();
    assert_eq!(gate.letters_introduced, 43);
    assert_eq!(gate.letters_chapters_met, 5);
    assert_eq!(gate.letters_active_days, 2);
    assert!(gate.satisfied);
    assert_eq!(gate.method, "organic");
}

#[test]
fn organic_gate_reports_current_values_and_all_targets() {
    let core = GreekCore::new_with_cards(temp_db_path("gate-progress"), None, combined_cards(true))
        .unwrap();
    let gate = core.get_alphabet_gate_state();
    assert_eq!(gate.letters_introduced, 0);
    assert_eq!(gate.letters_total, 43);
    assert_eq!(gate.letters_practice_total, 0);
    assert_eq!(gate.letters_practice_required, 20);
    assert_eq!(gate.letters_practice_accuracy, 0.0);
    assert_eq!(gate.letters_accuracy_required, 0.85);
    assert_eq!(gate.letters_active_days, 0);
    assert_eq!(gate.letters_active_days_required, 2);
    assert!(!gate.satisfied);
}
