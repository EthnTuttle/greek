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
    assert_eq!(v["wordsLearned"], 0, "fresh profile has no cards in review");
    // Fresh install: gate unsatisfied (letters deck loaded, no progress).
    assert_eq!(v["gateSatisfied"], false);
    assert_eq!(v["lettersChaptersMet"], 0);

    assert!(s.core.submit_review("letters-001".to_string(), 3));
    let after: serde_json::Value =
        serde_json::from_str(&commands::home_stats(&s.core).unwrap()).unwrap();
    assert_eq!(after["wordsLearned"], 1, "introduced card enters review count");
    std::fs::remove_dir_all(&s.data_dir).ok();
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

    // Easy schedules beyond the same-session window, so this test exercises
    // the plain pop path independently from short-step requeueing.
    let next = commands::review_rate(&s, 4).unwrap();
    assert_eq!(next["remaining"].as_u64().unwrap() + 1, remaining0);

    // Drain the session; the final payload is the completion shape and
    // rating beyond that errors.
    loop {
        let cur = commands::review_rate(&s, 4).unwrap();
        if cur["remaining"].as_u64().unwrap() == 0 {
            assert!(cur["card"].is_null());
            break;
        }
    }
    assert!(commands::review_rate(&s, 3).is_err());
    std::fs::remove_dir_all(&s.data_dir).ok();
}

#[test]
fn review_requeues_again_at_end_and_finishes_lone_card_honestly() {
    let s = fresh_state();
    let first = commands::review_current(&s).unwrap();
    let first_id = first["card"]["cardId"].as_str().unwrap().to_string();
    let remaining0 = first["remaining"].as_u64().unwrap();

    // Again is a one-minute learning step. The missed card goes BEHIND all
    // other content rather than becoming an instant one-card loop.
    let next = commands::review_rate(&s, 1).unwrap();
    assert_ne!(next["card"]["cardId"], first_id);
    assert_eq!(next["remaining"], remaining0);
    {
        let session = s.session.lock().unwrap();
        assert_eq!(session.queue.back().unwrap().card_id, first_id);
        assert_eq!(session.admitted, remaining0 as usize + 1);
    }

    // Pass the intervening cards. The missed card is seen again later in
    // this same batch, at the end of the queue.
    loop {
        let current = commands::review_current(&s).unwrap();
        if current["card"]["cardId"] == first_id {
            break;
        }
        commands::review_rate(&s, 4).unwrap();
    }
    assert_eq!(commands::review_current(&s).unwrap()["remaining"], 1);

    // With no other content, another Again ends the batch instead of tightly
    // re-serving the same card. The payload says why this is not "all done".
    let done = commands::review_rate(&s, 1).unwrap();
    assert!(done["card"].is_null());
    assert!(done["returningSoonCount"].as_u64().unwrap() >= 1);
    assert!(done["minutesUntilNext"].as_u64().unwrap() <= 1);
    std::fs::remove_dir_all(&s.data_dir).ok();
}

#[test]
fn review_requeue_respects_total_session_cap() {
    let s = fresh_state();
    let first = commands::review_current(&s).unwrap();
    let first_id = first["card"]["cardId"].as_str().unwrap().to_string();
    let remaining0 = first["remaining"].as_u64().unwrap();
    // Simulate the tail of a batch that has already admitted 20 + 40 cards.
    s.session.lock().unwrap().admitted = 60;
    let next = commands::review_rate(&s, 1).unwrap();
    assert_eq!(next["remaining"].as_u64().unwrap() + 1, remaining0);
    assert!(!s
        .session
        .lock()
        .unwrap()
        .queue
        .iter()
        .any(|c| c.card_id == first_id));
    assert_eq!(s.session.lock().unwrap().admitted, 60);
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
    for key in [
        "index",
        "title",
        "totalCards",
        "learningCount",
        "masteredCount",
        "unlocked",
        "hint",
    ] {
        assert!(ch.get(key).is_some(), "missing chapter field {key}");
    }
    std::fs::remove_dir_all(&s.data_dir).ok();
}

#[test]
fn declared_alphabet_command_unlocks_vocabulary_flow() {
    let s = fresh_state();
    let refused: serde_json::Value = serde_json::from_str(
        &commands::chapter_unlock(&s.core, "athenaze", 1).unwrap(),
    )
    .unwrap();
    assert_eq!(refused["unlocked"], false);

    let declared: serde_json::Value =
        serde_json::from_str(&commands::alphabet_declare_known(&s.core).unwrap()).unwrap();
    assert_eq!(declared["gateSatisfied"], true);
    let home: serde_json::Value =
        serde_json::from_str(&commands::home_stats(&s.core).unwrap()).unwrap();
    assert_eq!(home["gateSatisfied"], true);

    let unlocked: serde_json::Value = serde_json::from_str(
        &commands::chapter_unlock(&s.core, "athenaze", 1).unwrap(),
    )
    .unwrap();
    assert_eq!(unlocked["unlocked"], true);
    assert!(!s
        .core
        .get_new_cards("athenaze".to_string(), 1, 20)
        .is_empty());
    std::fs::remove_dir_all(&s.data_dir).ok();
}

#[test]
fn alphabet_check_command_loop_latches_gate() {
    let s = fresh_state();

    // Gate freshly unsatisfied: the core refuses vocabulary-track unlocks.
    let refused: serde_json::Value = serde_json::from_str(
        &commands::chapter_unlock(&s.core, "athenaze", 1).unwrap(),
    )
    .unwrap();
    assert_eq!(refused["unlocked"], false, "gate unmet → vocab unlock refused");
    assert!(refused["hint"].as_str().unwrap().contains("Alphabet"));

    // The app's check flow: one generation, per-question answers, one verdict.
    let qs: Vec<serde_json::Value> =
        serde_json::from_str(&commands::alphabet_check_generate(&s.core).unwrap()).unwrap();
    assert_eq!(qs.len(), 24, "ALPHABET_CHECK_N is 24");
    for q in &qs {
        assert_eq!(q["choices"].as_array().unwrap().len(), 4);
        assert!(q["cardId"].as_str().unwrap().starts_with("letters-"));
        assert!(q["correctIndex"].as_u64().unwrap() < 4);
        for key in ["glyphToName", "prompt"] {
            assert!(q.get(key).is_some(), "missing question field {key}");
        }
    }

    // A failing verdict does NOT latch the gate.
    let fail: serde_json::Value =
        serde_json::from_str(&commands::alphabet_check_verdict(&s.core, 10, 24).unwrap()).unwrap();
    assert_eq!(fail["gateSatisfied"], false, "10/24 must not pass");

    // Answer every generated question correctly (the UI scores locally via
    // correctIndex; here the harness plays a perfect student).
    for q in &qs {
        let ok: String = serde_json::from_str(
            &commands::alphabet_check_answer(&s.core, q["cardId"].as_str().unwrap(), true)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(ok, "ok");
    }
    let verdict: serde_json::Value =
        serde_json::from_str(&commands::alphabet_check_verdict(&s.core, 24, 24).unwrap()).unwrap();
    assert_eq!(verdict["gateSatisfied"], true, "24/24 latches the gate");

    // Gate satisfied: vocabulary unlocks are accepted, curriculum agrees.
    let ok: serde_json::Value = serde_json::from_str(
        &commands::chapter_unlock(&s.core, "athenaze", 1).unwrap(),
    )
    .unwrap();
    assert_eq!(ok["unlocked"], true, "gate met → vocab unlock accepted");
    let cur: serde_json::Value =
        serde_json::from_str(&commands::curriculum(&s.core).unwrap()).unwrap();
    assert_eq!(cur["gateNotMet"], false);
    std::fs::remove_dir_all(&s.data_dir).ok();
}

/// D1 regression, updated for core pacing: reviewing the seven vowels makes
/// them LEARNED, not 30-day mature. Curriculum::get_chapters now persists the
/// earned Letters ch.2 unlock automatically; the next one-pass session build
/// can therefore serve non-vowel cards without a hidden manual dead end.
#[test]
fn d1_letters_ch2_auto_unlock_yields_non_vowel_cards() {
    let s = fresh_state();

    // Audit repro state: seven vowels seen twice → learning 7/7, maturity 0/7.
    for n in 1..=7u32 {
        let id = format!("letters-{n:03}");
        assert!(s.core.submit_review(id.clone(), 3));
        assert!(s.core.submit_review(id, 3));
    }
    let cur: serde_json::Value =
        serde_json::from_str(&commands::curriculum(&s.core).unwrap()).unwrap();
    let books = cur["textbooks"].as_array().unwrap();
    let letters = books.iter().find(|b| b["id"] == "letters").unwrap();
    assert_eq!(letters["chapters"][0]["learningCount"], 7, "7/7 vowels learning");
    assert_eq!(
        letters["chapters"][0]["masteredCount"], 0,
        "fresh low-stability vowels are learned, not long-term mastered"
    );
    let ch2 = &letters["chapters"][1];
    assert_eq!(ch2["unlocked"], true, "earned unlock is persisted by the core");

    // The explicit command remains harmless for existing UI callers.
    let r: serde_json::Value =
        serde_json::from_str(&commands::chapter_unlock(&s.core, "letters", 2).unwrap()).unwrap();
    assert_eq!(r["unlocked"], true, "Letters ch.2 stays unlocked");

    // The next session build must now serve non-vowel cards (letters ch.2 =
    // stops). Daily cap check: 7 introduced today, 3 remain — enough to see one.
    let session = s.core.build_review_session(20).expect("build_review_session");
    assert!(
        session
            .iter()
            .any(|c| c.card.id.starts_with("letters-") && c.card.chapter == 2),
        "next session must contain Letters ch.2 cards, got: {:?}",
        session
            .iter()
            .map(|c| c.card.id.clone())
            .collect::<Vec<_>>()
    );
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

/// D2: the UI must be able to tell "no lexicon installed" apart from
/// "lexicon present, zero hits". fresh_state stages no morphology, so
/// installed must be false here; resolution-order mechanics (bundle dir
/// first after the env override) are covered by state.rs unit tests.
#[test]
fn dict_status_reports_installation() {
    let s = fresh_state();
    let v: serde_json::Value =
        serde_json::from_str(&commands::dict_status(&s).unwrap()).unwrap();
    assert_eq!(v["installed"], false, "no morphology staged in fresh_state");
    assert!(v["path"].is_null(), "no path when not installed");
    assert!(!s.dictionary_installed());
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
    assert_eq!(batch["v"], 2, "raw SyncBatch JSON (not double-encoded)");

    // Importing our own backup: events are uuid-deduped, so 0 applied.
    let res: serde_json::Value =
        serde_json::from_str(&commands::backup_import(&s, &exported).unwrap()).unwrap();
    assert_eq!(res["reviewsReceived"], 0);

    // A fresh device importing the same backup DOES receive the review.
    let s2 = fresh_state();
    let res2: serde_json::Value =
        serde_json::from_str(&commands::backup_import(&s2, &exported).unwrap()).unwrap();
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

/// D8 regression (audit repro): an open review batch must NOT survive an
/// import. Device A opens a session (caching its current card as "new");
/// device B reviews that exact card; A imports B's backup → the cached
/// batch is stale and would re-offer the card as new. The fix invalidates
/// it, so the rebuilt batch never shows that card as new again.
#[test]
fn d8_backup_import_invalidates_cached_review_session() {
    let a = fresh_state();
    // A opens a session: the in-memory batch is now cached.
    let open = commands::review_current(&a).unwrap();
    let card_id = open["card"]["cardId"]
        .as_str()
        .expect("fresh install offers a first card")
        .to_string();
    assert_eq!(open["isNew"], true);

    // B (another device) reviews THAT card, then exports.
    let b = fresh_state();
    assert!(b.core.submit_review(card_id.clone(), 4));
    let backup = commands::backup_export(&b.core).unwrap();

    // A imports the backup — the card is now reviewed elsewhere…
    let res: serde_json::Value =
        serde_json::from_str(&commands::backup_import(&a, &backup).unwrap()).unwrap();
    assert!(
        res["reviewsReceived"].as_u64().unwrap() >= 1,
        "the import must carry B's review of {card_id}"
    );

    // …so A's next review_current must NOT re-offer it as new. The
    // invalidated batch rebuilds and the card (no longer new, not due) is
    // simply gone from the head of the queue.
    let after = commands::review_current(&a).unwrap();
    match after["card"]["cardId"].as_str() {
        Some(id) if id == card_id => {
            assert!(
                !after["isNew"].as_bool().unwrap_or(true),
                "{card_id} was reviewed on B; A must not re-offer it as new (stale session survived import)"
            );
        }
        _ => {} // different card or empty queue — both fine
    }
    std::fs::remove_dir_all(&a.data_dir).ok();
    std::fs::remove_dir_all(&b.data_dir).ok();
}

/// D3 regression (audit P1 S1, core-side fix verified from desktop):
/// backup export → fresh-profile restore must carry the FULL user state,
/// not just review logs: device A adds a Greek reader-lookup card, reviews
/// it, unlocks a chapter and marks a word known → export → fresh profile B
/// import → the review is folded in, plus the lookup content row, the
/// chapter unlock and the known word are all present on B.
///
/// Desktop has no lookup command (the reader is Android-only for now), so
/// the lookup-path setup drives GreekCore directly, exactly as the Android
/// "Add to SRS" button does.
#[test]
fn d3_backup_restore_carries_full_user_state() {
    let a = fresh_state();

    // Reader lookup path: "Add to SRS" from a tapped word.
    let card_id = a
        .core
        .add_card_from_lookup(
            "λόγος".to_string(),
            "λόγος".to_string(),
            "word, reason".to_string(),
        )
        .expect("add_card_from_lookup");
    assert_eq!(card_id, "lookup-λόγος");
    assert!(a.core.submit_review(card_id.clone(), 3), "lookup card reviewed");

    // Chapter unlock: learn the 7 vowels so Letters ch.2 unlocks (D1 flow).
    for n in 1..=7u32 {
        let id = format!("letters-{n:03}");
        assert!(a.core.submit_review(id.clone(), 3));
        assert!(a.core.submit_review(id, 3));
    }
    let unlocked: serde_json::Value = serde_json::from_str(
        &commands::chapter_unlock(&a.core, "letters", 2).unwrap(),
    )
    .unwrap();
    assert_eq!(unlocked["unlocked"], true, "A unlocks Letters ch.2");

    a.core
        .mark_word_known("καλός".to_string(), true)
        .expect("mark_word_known");

    let backup = commands::backup_export(&a.core).unwrap();
    let batch: serde_json::Value = serde_json::from_str(&backup).unwrap();
    assert_eq!(batch["v"], 2, "v2 wire format carries the full state");

    // Fresh profile B imports.
    let b = fresh_state();
    let res: serde_json::Value =
        serde_json::from_str(&commands::backup_import(&b, &backup).unwrap()).unwrap();
    assert!(res["reviewsReceived"].as_u64().unwrap() >= 1);

    // 1. Review folded in: B's stats see the reviews A made today.
    let stats = b.core.get_review_stats();
    assert!(
        stats.reviewed_today >= 1,
        "B must fold A's reviews, got reviewed_today={}",
        stats.reviewed_today
    );

    // 2. Lookup content row present on B (the card's front/back survive —
    //    without it the folded review points at a card with no content).
    //    B's own re-export is the readable projection of its lookup_cards.
    let re_export: serde_json::Value =
        serde_json::from_str(&commands::backup_export(&b.core).unwrap()).unwrap();
    let lookup_cards = re_export["lookup_cards"]
        .as_array()
        .expect("v2 re-export has lookup_cards");
    let row = lookup_cards
        .iter()
        .find(|c| c["card_id"] == card_id)
        .unwrap_or_else(|| panic!("B must hold the lookup content row for {card_id}"));
    assert_eq!(row["front"], "λόγος");
    assert_eq!(
        row["back"], "λόγος\nword, reason",
        "lookup review content carries the lemma alongside its gloss"
    );

    // 3. Chapter unlock present: B never learned the vowels, yet ch.2 is
    //    open because the unlock row travelled in the backup.
    let cur: serde_json::Value =
        serde_json::from_str(&commands::curriculum(&b.core).unwrap()).unwrap();
    let letters = cur["textbooks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "letters")
        .unwrap();
    assert_eq!(
        letters["chapters"][1]["unlocked"], true,
        "B must inherit A's Letters ch.2 unlock"
    );

    // 4. Known word present.
    assert!(
        b.core.is_word_known("καλός".to_string()),
        "B must inherit A's known-word mark"
    );
    std::fs::remove_dir_all(&a.data_dir).ok();
    std::fs::remove_dir_all(&b.data_dir).ok();
}

/// D6: the Home "New today" counter is fed by the REAL core budget
/// (new_cards_today + new_cards_remaining_today), the same single-snapshot
/// budget build_review_session consumes — so the counter and the session
/// fill can never disagree (the audit's "20/20 against a 10-cap" bug was
/// the old 20/10 over-admission + hardcoded label, fixed at the API level).
#[test]
fn d6_home_budget_matches_session_admission() {
    let s = fresh_state();

    let before: serde_json::Value =
        serde_json::from_str(&commands::home_stats(&s.core).unwrap()).unwrap();
    assert_eq!(before["newCap"], 10, "cap derived from the core, not hardcoded 20");
    let remaining = before["newCap"].as_u64().unwrap() - before["newToday"].as_u64().unwrap();

    // A session started on fresh install is letters-only: those are exempt
    // from the cap, unlike vocabulary cards below.
    let session = commands::fill_session(&s.core);
    let admitted_new = session.iter().filter(|c| c.is_new).count() as u64;
    assert!(
        admitted_new <= remaining,
        "session admitted {admitted_new} new cards against a remaining budget of {remaining}"
    );
    assert!(
        admitted_new <= 10,
        "the 10-cap must hold even when 20 session slots are free"
    );

    // Study the whole batch: letters are cap-exempt now (owner decision),
    // so Home's vocabulary-only counter must NOT change.
    for c in &session {
        assert!(s.core.submit_review(c.card_id.clone(), 3));
    }
    let after: serde_json::Value =
        serde_json::from_str(&commands::home_stats(&s.core).unwrap()).unwrap();
    assert_eq!(after["newCap"], 10, "cap stays core-enforced after studying");
    assert_eq!(
        after["newToday"].as_u64().unwrap(),
        before["newToday"].as_u64().unwrap(),
        "letters must not consume the vocabulary-only newToday counter"
    );

    // The admitted cards are now ineligible as "new". Other still-unseen
    // cards may be admitted with any remaining daily budget (notably the
    // next letters chapter auto-unlocks), but an introduced ID never returns
    // as new the same day.
    let next = commands::fill_session(&s.core);
    assert!(
        !next.iter().any(|next_card| {
            next_card.is_new
                && session
                    .iter()
                    .any(|studied| studied.card_id == next_card.card_id)
        }),
        "an introduced card must not be re-served as new the same day"
    );
    std::fs::remove_dir_all(&s.data_dir).ok();
}

/// Vocabulary cap (letters exempt): with the alphabet declared known, a fill
/// draws the ungated letters then at most `remaining` NEW vocabulary cards.
#[test]
fn d6_vocabulary_budget_matches_session_admission_with_letters_exempt() {
    let s = fresh_state();
    s.core.declare_alphabet_known();
    let before: serde_json::Value =
        serde_json::from_str(&commands::home_stats(&s.core).unwrap()).unwrap();
    let remaining = before["newCap"].as_u64().unwrap() - before["newToday"].as_u64().unwrap();

    let session = commands::fill_session(&s.core);
    let new_vocab: Vec<_> = session
        .iter()
        .filter(|c| c.is_new && !c.card_id.starts_with("letters-"))
        .collect();
    assert!(
        (new_vocab.len() as u64) <= remaining,
        "vocabulary admitted {} new against remaining {remaining}",
        new_vocab.len()
    );
    assert!(
        !new_vocab.is_empty(),
        "declared gate must let the vocabulary test exercise the cap"
    );
    for c in &session {
        assert!(s.core.submit_review(c.card_id.clone(), 3));
    }
    let after: serde_json::Value =
        serde_json::from_str(&commands::home_stats(&s.core).unwrap()).unwrap();
    assert_eq!(
        after["newToday"].as_u64().unwrap(),
        before["newToday"].as_u64().unwrap() + new_vocab.len() as u64,
        "newToday must count exactly the vocabulary cards admitted"
    );
    assert_eq!(after["newCap"].as_u64().unwrap(), 10);
    std::fs::remove_dir_all(&s.data_dir).ok();
}
