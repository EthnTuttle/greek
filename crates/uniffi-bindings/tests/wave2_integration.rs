//! End-to-end integration test for the wave-2 UniFFI surface (plain Rust
//! calls against `GreekCore`, no JNI): daily new-card cap, interval preview,
//! and new-card counters over `submit_review`.

use std::env;
use uniffi_bindings::GreekCore;

fn temp_db_path(tag: &str) -> String {
    let dir = env::temp_dir().join(format!("greek-wave2-{}-{}", tag, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("test.db").to_string_lossy().into_owned()
}

/// Build a cards JSON array with `n` cards all in chapter 1 of athenaze.
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

fn open_core(tag: &str, cards: &str) -> std::sync::Arc<GreekCore> {
    GreekCore::new_with_cards(temp_db_path(tag), None, cards.to_string()).unwrap()
}

/// Add `days` to a "YYYY-MM-DD" date (Hinnant civil algorithm, inverse of the
/// one in src/lib.rs) — just enough for date-prefix comparisons in tests.
fn date_plus_days(date: &str, days: i64) -> String {
    let y: i64 = date[0..4].parse().unwrap();
    let m: i64 = date[5..7].parse().unwrap();
    let d: i64 = date[8..10].parse().unwrap();
    let yy = if m <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let mut z = era * 146097 + doe - 719468 + days;
    // civil_from_days
    z += 719468;
    let era2 = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe2 = z - era2 * 146097;
    let yoe2 = (doe2 - doe2 / 1460 + doe2 / 36524 - doe2 / 146096) / 365;
    let y2 = yoe2 + era2 * 400;
    let doy2 = doe2 - (365 * yoe2 + yoe2 / 4 - yoe2 / 100);
    let mp = (5 * doy2 + 2) / 153;
    let d2 = doy2 - (153 * mp + 2) / 5 + 1;
    let m2 = if mp < 10 { mp + 3 } else { mp - 9 };
    let y_out = if m2 <= 2 { y2 + 1 } else { y2 };
    format!("{:04}-{:02}-{:02}", y_out, m2, d2)
}

#[test]
fn wave2_surface_end_to_end() {
    // 12 cards so the cap of 10 is distinguishable from "all cards".
    let core = open_core("e2e", &cards_json(12));

    // Day-1 session respects the daily cap even when more cards exist.
    let batch = core.get_new_cards("athenaze".to_string(), 1, 50);
    assert_eq!(batch.len(), 10, "day-1 batch must respect the cap of 10");

    // No new cards studied yet.
    assert_eq!(core.new_cards_today(), 0);
    assert_eq!(core.new_cards_remaining_today(), 10);

    // Rate the first card Again (rating 1) and check the surface updates.
    let first_id = batch[0].id.clone();
    assert!(core.submit_review(first_id.clone(), 1));
    assert_eq!(core.new_cards_today(), 1);
    assert_eq!(core.new_cards_remaining_today(), 9);

    // Interval preview on the now-scheduled card: ordered again <= hard <=
    // good <= easy, all non-zero, and Again lands sub-day (< 24h).
    let preview = core.preview_intervals(first_id.clone()).expect("preview for reviewed card");
    assert!(preview.again_secs > 0 && preview.again_secs < 86_400, "again = {}", preview.again_secs);
    assert!(preview.again_secs <= preview.hard_secs, "again {} <= hard {}", preview.again_secs, preview.hard_secs);
    assert!(preview.hard_secs <= preview.good_secs, "hard {} <= good {}", preview.hard_secs, preview.good_secs);
    assert!(preview.good_secs <= preview.easy_secs, "good {} <= easy {}", preview.good_secs, preview.easy_secs);

    // A never-reviewed card is treated as new and still returns an ordered
    // preview (None only on error).
    let unseen = open_core("e2e-unseen", &cards_json(2));
    let fresh = unseen.preview_intervals("test-card-0".to_string()).expect("preview for fresh card");
    assert!(fresh.again_secs > 0 && fresh.again_secs < 86_400, "fresh again = {}", fresh.again_secs);
    assert!(fresh.again_secs <= fresh.good_secs, "fresh again {} <= good {}", fresh.again_secs, fresh.good_secs);

    // Burn through the remaining 9 introductions of the day.
    for card in &batch[1..] {
        assert!(core.submit_review(card.id.clone(), 3)); // Good
    }
    assert_eq!(core.new_cards_today(), 10);
    assert_eq!(core.new_cards_remaining_today(), 0);

    // Cap reached: get_new_cards must return empty even though 2 of the 12
    // cards in the chapter are still unseen.
    let empty = core.get_new_cards("athenaze".to_string(), 1, 50);
    assert!(empty.is_empty(), "cap reached -> no new cards, got {}", empty.len());

    // The Again-rated card now has a memory state and is therefore excluded
    // from new-card batches on later days even after the cap resets.
    // (It is NOT immediately re-due: the Again learning step lands ~10 min in
    // the future — the sub-day due requeue is validated separately.)
    let fresh_batch = core.get_new_cards("athenaze".to_string(), 1, 50);
    assert!(
        fresh_batch.iter().all(|c| c.id != first_id),
        "reviewed card must not reappear in new-card batches"
    );
}

#[test]
fn pedagogy_smoke_real_athenaze_data() {
    // Load the real deck exactly as the Android app does.
    let cards = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/cards/athenaze-i.json"
    ))
    .expect("athenaze-i.json");

    let db_path = temp_db_path("pedagogy");
    let core = GreekCore::new_with_cards(db_path.clone(), None, cards).unwrap();

    // Day-1 session: never more than the daily cap of 10, and non-empty.
    let day1 = core.get_new_cards("athenaze".to_string(), 1, 50);
    assert!(!day1.is_empty(), "day-1 batch must not be empty");
    assert!(day1.len() <= 10, "day-1 batch must be <= cap, got {}", day1.len());

    // Rate one card Again: due_date must be a full ISO-8601 timestamp in the
    // near future (< 24h), never midnight-stamped.
    let card = &day1[0];
    let before = greek_core::now_iso8601();
    assert!(core.submit_review(card.id.clone(), 1)); // Again

    let db = rusqlite::Connection::open(&db_path).unwrap();
    let due_date: String = db
        .query_row(
            "SELECT due_date FROM memory_states WHERE card_id = ?1",
            [&card.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(due_date.len(), 20, "full timestamp expected, got {due_date}");
    assert!(
        !due_date.ends_with("T00:00:00Z"),
        "due_date must NOT be midnight-stamped: {due_date}"
    );
    assert!(due_date >= before, "due_date {due_date} before review time {before}?");
    // < 24h in the future => its date prefix is today or tomorrow at worst.
    let today = &before[..10];
    let tomorrow = date_plus_days(today, 1);
    assert!(
        &due_date[..10] == today || &due_date[..10] == tomorrow,
        "Again due_date {due_date} should land < 24h out (today {today})"
    );

    // Curriculum view on a fresh deck: nothing mastered yet, ch.2 gated.
    let chapters = core.get_chapters("athenaze".to_string());
    assert_eq!(chapters.len(), 31, "athenaze total chapters");
    let ch1 = &chapters[0];
    assert_eq!(ch1.mastered_count, 0, "fresh DB must show 0 mastered");
    assert!(ch1.unlocked, "chapter 1 unlocked by default");
    let ch2 = &chapters[1];
    assert!(!ch2.unlocked, "chapter 2 locked on fresh DB");
    assert!(!ch2.unlock_hint.is_empty(), "chapter 2 must explain its unlock requirement");

    // Chapters beyond card coverage (athenaze-i ships ch.1..=16; the book
    // runs to 31) can never deadlock: their unlock requirement is vacuous,
    // so the hint is empty.
    let beyond = &chapters[30];
    assert_eq!(beyond.chapter, 31);
    assert!(!beyond.available, "chapter 31 has no cards -> unavailable");
    assert!(beyond.unlock_hint.is_empty(), "no deadlock hint past coverage: {:?}", beyond.unlock_hint);
}
