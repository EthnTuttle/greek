//! Publisher/flush behavior against scripted fake relays (OK-false prefix
//! fixtures, mid-publish socket close, pre-seeded stores for read-back).

mod common;

use common::{EventScript, FakeRelay};
use greek_core::Database;
use share_nostr::caps;
use share_nostr::identity::ShareKeys;
use share_nostr::ops::{self, EnqueueOutcome};
use share_nostr::outbox::{self, OutboxState};
use share_nostr::publisher::Publisher;
use share_nostr::templates::{self, TemplateLang};

fn db_enabled() -> Database {
    let db = Database::open(":memory:", None).unwrap();
    greek_core::sync::setting_put(&db, ops::ENABLED_KEY, "1").unwrap();
    db
}

fn keys() -> ShareKeys {
    ShareKeys::derive_from_sync(&[0xAB; 32]).unwrap()
}

/// ~day 20370 (UTC), one hour after the day boundary, giving flush tests
/// 23 hours of same-day runway.
const T0: i64 = 1_759_968_000_000 + 3_600_000;
const HOUR: i64 = 3_600_000;

async fn publisher(keys: &ShareKeys, relay: &FakeRelay) -> Publisher {
    Publisher::connect(keys, &[relay.url.clone()])
        .await
        .expect("connect to fake relay")
}

async fn enqueue_streak7(db: &Database, keys: &ShareKeys, dedupe: &str, now_ms: i64) {
    let outcome = ops::enqueue_milestone(
        db,
        templates::SLUG_STREAK_7,
        dedupe,
        dedupe,
        TemplateLang::En,
        Some(7),
        None,
        keys,
        now_ms,
    )
    .unwrap();
    assert_eq!(outcome, EnqueueOutcome::Enqueued, "test enqueues must succeed");
}

#[tokio::test]
async fn flush_posts_exactly_once_and_second_flush_is_idempotent() {
    let db = db_enabled();
    let keys = keys();
    let relay = FakeRelay::start(EventScript::AcceptAll).await;
    let publisher = publisher(&keys, &relay).await;

    enqueue_streak7(&db, &keys, "streak.7:2025-08-01", T0).await;
    let r1 = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r1.posted, 1);
    assert_eq!(r1.deferred + r1.duplicates, 0);
    assert!(r1.failed.is_empty());
    assert_eq!(relay.received_count(), 1);

    // The posted milestone is recorded with a marker + per-relay acceptance.
    let marker = caps::read_marker(&db, "streak.7:2025-08-01").unwrap().unwrap();
    assert!(!marker.event_id.is_empty());
    assert_eq!(marker.relays.len(), 1);
    let row = outbox::get(&db, "streak.7:2025-08-01").unwrap().unwrap();
    assert_eq!(row.state, OutboxState::Posted);

    // Double flush: no new wire traffic, no state churn.
    let r2 = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r2, Default::default());
    assert_eq!(relay.received_count(), 1, "relay must see the event exactly once");

    // The content on the wire is exactly the shipped EN preset.
    let stored = relay.stored();
    assert_eq!(
        stored[0]["content"].as_str().unwrap(),
        templates::render(
            templates::preset(templates::SLUG_STREAK_7, TemplateLang::En).unwrap(),
            Some(7),
            None,
        )
        .unwrap()
    );
    publisher.disconnect().await;
    relay.stop().await;
}

#[tokio::test]
async fn duplicate_prefix_is_treated_as_delivered() {
    let db = db_enabled();
    let keys = keys();
    let relay = FakeRelay::start(EventScript::Reject("duplicate: already have this event".into())).await;
    let publisher = publisher(&keys, &relay).await;

    enqueue_streak7(&db, &keys, "dup:1", T0).await;
    let report = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(report.duplicates, 1, "duplicate OK must count delivered");
    assert_eq!(report.posted, 0);
    let row = outbox::get(&db, "dup:1").unwrap().unwrap();
    assert_eq!(row.state, OutboxState::Posted);
    let marker = caps::read_marker(&db, "dup:1").unwrap().unwrap();
    assert!(!marker.event_id.is_empty());
    publisher.disconnect().await;
    relay.stop().await;
}

#[tokio::test]
async fn rate_limited_row_defers_and_succeeds_on_retry() {
    let db = db_enabled();
    let keys = keys();
    let relay = FakeRelay::start(EventScript::Reject("rate-limited: slow down".into())).await;
    let publisher = publisher(&keys, &relay).await;

    enqueue_streak7(&db, &keys, "rl:1", T0).await;
    let r1 = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r1.deferred, 1);
    assert_eq!(r1.posted, 0);
    assert!(
        r1.failed.iter().any(|(_, reason)| reason.contains("rate-limited")),
        "relay failure must surface in the report: {r1:?}"
    );
    assert_eq!(outbox::get(&db, "rl:1").unwrap().unwrap().state, OutboxState::Queued);

    // The relay relents (next flush after backoff): same signed bytes, now
    // accepted.
    relay.set_script(EventScript::AcceptAll);
    let r2 = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r2.posted, 1);
    assert_eq!(outbox::get(&db, "rl:1").unwrap().unwrap().state, OutboxState::Posted);
    publisher.disconnect().await;
    relay.stop().await;
}

#[tokio::test]
async fn permanent_prefixes_fail_the_row_permanently() {
    for prefix in [
        "blocked: nice try",
        "invalid: something rotten",
        "pow: need difficulty 24",
        "restricted: not in the allowlist",
    ] {
        let db = db_enabled();
        let keys = keys();
        let relay = FakeRelay::start(EventScript::Reject(prefix.to_string())).await;
        let publisher = publisher(&keys, &relay).await;

        enqueue_streak7(&db, &keys, "p:1", T0).await;
        let report = ops::flush(&db, &publisher, T0).await.unwrap();
        assert_eq!(report.posted + report.duplicates, 0, "prefix {prefix:?}");
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].1.contains(prefix), "{prefix}");
        assert_eq!(
            outbox::get(&db, "p:1").unwrap().unwrap().state,
            OutboxState::FailedPerma,
            "prefix {prefix:?} is a permanent stop"
        );
        // Nothing is deferred forever: a repeat flush is a no-op.
        let r2 = ops::flush(&db, &publisher, T0).await.unwrap();
        assert_eq!(r2, Default::default());
        publisher.disconnect().await;
        relay.stop().await;
    }
}

#[tokio::test]
async fn auth_required_is_permanent_unsupported_in_v1() {
    let db = db_enabled();
    let keys = keys();
    let relay =
        FakeRelay::start(EventScript::Reject("auth-required: we only serve authed users".into()))
            .await;
    let publisher = publisher(&keys, &relay).await;
    enqueue_streak7(&db, &keys, "auth:1", T0).await;
    let report = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(report.posted + report.duplicates + report.deferred, 0);
    assert!(report.failed[0].1.contains("auth-required"));
    assert_eq!(
        outbox::get(&db, "auth:1").unwrap().unwrap().state,
        OutboxState::FailedPerma,
        "NIP-42 auth relays are permanent-unsupported for v1"
    );
    publisher.disconnect().await;
    relay.stop().await;
}

#[tokio::test]
async fn socket_close_mid_publish_keeps_row_queued_without_panicking() {
    let db = db_enabled();
    let keys = keys();
    let relay = FakeRelay::start(EventScript::CloseOnEvent).await;
    let publisher = publisher(&keys, &relay).await;
    enqueue_streak7(&db, &keys, "close:1", T0).await;
    let report = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(report.posted, 0);
    assert_eq!(report.deferred, 1, "mid-publish death is transient");
    assert_eq!(
        outbox::get(&db, "close:1").unwrap().unwrap().state,
        OutboxState::Queued
    );
    // The relay heals; the row publishes on a later flush.
    relay.set_script(EventScript::AcceptAll);
    let r2 = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r2.posted, 1, "queued row retries identically: {r2:?}");
    publisher.disconnect().await;
    relay.stop().await;
}

#[tokio::test]
async fn partial_relay_acceptance_retries_then_converges() {
    let db = db_enabled();
    let keys = keys();
    let accepting = FakeRelay::start(EventScript::AcceptAll).await;
    let limiting = FakeRelay::start(EventScript::Reject("rate-limited: max 30/h".into())).await;
    let publisher =
        Publisher::connect(&keys, &[accepting.url.clone(), limiting.url.clone()])
            .await
            .unwrap();

    enqueue_streak7(&db, &keys, "multi:1", T0).await;
    let r1 = ops::flush(&db, &publisher, T0).await.unwrap();
    // One relay delivered, the other is transient-retrying: row stays queued
    // with a merged per-relay map.
    assert_eq!(r1.deferred, 1, "partial delivery defers: {r1:?}");
    let row = outbox::get(&db, "multi:1").unwrap().unwrap();
    assert_eq!(row.state, OutboxState::Queued);
    let map = row.per_relay_map();
    assert_eq!(map.len(), 2);
    assert_eq!(
        map.values().filter(|v| v.as_str() == "accepted").count(),
        1,
        "accepting relay recorded: {map:?}"
    );
    assert!(map.values().any(|v| v.starts_with("failed:rate-limited")));

    // Second flush: pre-publish read-back sees the accepting relay already
    // serving the content → converged without re-sending.
    let r2 = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r2.duplicates, 1, "read-back convergence: {r2:?}");
    assert_eq!(outbox::get(&db, "multi:1").unwrap().unwrap().state, OutboxState::Posted);
    // The accepting relay saw the event exactly once (no redundant resend).
    assert_eq!(accepting.received_count(), 1);
    publisher.disconnect().await;
    accepting.stop().await;
    limiting.stop().await;
}

#[tokio::test]
async fn pre_existing_content_marks_posted_without_resending() {
    let db = db_enabled();
    let keys = keys();
    let relay = FakeRelay::start(EventScript::AcceptAll).await;
    let publisher = publisher(&keys, &relay).await;

    enqueue_streak7(&db, &keys, "rb:1", T0).await;
    // Another publisher (e.g. our second device) already delivered the same
    // content: seed the relay's store with the exact event json.
    let row = outbox::get(&db, "rb:1").unwrap().unwrap();
    relay.seed_stored(serde_json::from_str(&row.event_json).unwrap());
    let before = relay.received_count();

    let report = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(report.duplicates, 1);
    assert_eq!(report.posted, 0);
    assert_eq!(
        relay.received_count(),
        before,
        "read-back hit ⇒ nothing resent"
    );
    assert_eq!(outbox::get(&db, "rb:1").unwrap().unwrap().state, OutboxState::Posted);
    publisher.disconnect().await;
    relay.stop().await;
}

#[tokio::test]
async fn oversized_content_fails_publish_time_validation_before_the_wire() {
    let db = db_enabled();
    let keys = keys();
    let relay = FakeRelay::start(EventScript::AcceptAll).await;
    let publisher = publisher(&keys, &relay).await;

    // Bypass enqueue_milestone's save-time validation to simulate a poisoned
    // outbox row (e.g. synced/hand-edited DB).
    let big = "α".repeat(600); // 1200 bytes > 1024 cap
    let ev = keys.sign_note(&big).unwrap();
    outbox::enqueue(
        &db,
        "too-big",
        templates::SLUG_CHAPTER_MASTERED,
        &serde_json::to_string(&ev).unwrap(),
        "ath/1",
        T0,
    )
    .unwrap();
    let report = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(report.posted, 0);
    assert!(report
        .failed
        .iter()
        .any(|(who, why)| who == "local" && why.contains("1024")))
    ;
    assert_eq!(outbox::get(&db, "too-big").unwrap().unwrap().state, OutboxState::FailedPerma);
    assert_eq!(relay.received_count(), 0, "oversized content must never hit the wire");
    publisher.disconnect().await;
    relay.stop().await;
}

#[tokio::test]
async fn disabled_is_zero_network_activity() {
    let db = Database::open(":memory:", None).unwrap(); // share.enabled absent
    let keys = keys();
    let relay = FakeRelay::start(EventScript::AcceptAll).await;
    // enqueue_milestone refuses while disabled…
    let outcome = ops::enqueue_milestone(
        &db,
        templates::SLUG_STREAK_7,
        "off:1",
        "off:1",
        TemplateLang::En,
        Some(7),
        None,
        &keys,
        T0,
    )
    .unwrap();
    assert_eq!(outcome, EnqueueOutcome::Disabled);
    // …and even a hand-seeded queued row is never touched by flush.
    let ev_raw = ShareKeys::derive_from_sync(&[0xAB; 32]).unwrap().sign_note("x").unwrap();
    outbox::enqueue(&db, "off:2", "streak.7", &serde_json::to_string(&ev_raw).unwrap(), "v", T0)
        .unwrap();
    let publisher = publisher(&keys, &relay).await; // connecting is allowed; sharing traffic is not
    let report = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(report, Default::default());
    assert_eq!(relay.received_count(), 0);
    assert_eq!(relay.req_count(), 0, "no read-back REQs either");
    assert_eq!(outbox::get(&db, "off:2").unwrap().unwrap().state, OutboxState::Queued);
    publisher.disconnect().await;
    relay.stop().await;
}

#[tokio::test]
async fn flush_caps_spacing_and_daily_overflow() {
    let db = db_enabled();
    let keys = keys();
    let relay = FakeRelay::start(EventScript::AcceptAll).await;
    let publisher = publisher(&keys, &relay).await;

    for (i, key) in ["d1", "d2", "d3"].iter().enumerate() {
        ops::enqueue_milestone(
            &db,
            templates::SLUG_CHAPTER_MASTERED,
            key,
            key,
            TemplateLang::En,
            None,
            Some(&format!("suffix {i}")),
            &keys,
            T0,
        )
        .unwrap();
    }
    // First flush: posts the oldest, spacing-defers the rest.
    let r1 = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r1.posted, 1);
    assert_eq!(r1.deferred, 2);
    // Six hours later: the second post is allowed by spacing… and spends
    // the last slot of the 2/day budget, so the third row overflows into
    // skipped_cap inside the same flush (deferring it would be dishonest:
    // budget-wise there is no "later today" left).
    let r2 = ops::flush(&db, &publisher, T0 + 6 * HOUR).await.unwrap();
    assert_eq!(r2.posted, 1);
    assert_eq!(r2.deferred, 0);
    assert_eq!(
        outbox::get(&db, "d3").unwrap().unwrap().state,
        OutboxState::SkippedCap,
        "daily overflow becomes a permanent skipped_cap history row"
    );
    // Twelve hours on: nothing left to do.
    let r3 = ops::flush(&db, &publisher, T0 + 12 * HOUR).await.unwrap();
    assert_eq!(r3, Default::default());
    assert_eq!(relay.received_count(), 2);

    // A milestone enqueued after the day's budget is gone skips at enqueue.
    let outcome = ops::enqueue_milestone(
        &db,
        templates::SLUG_STREAK_7,
        "d4",
        "d4",
        TemplateLang::En,
        Some(7),
        None,
        &keys,
        T0 + 12 * HOUR,
    )
    .unwrap();
    assert_eq!(outcome, EnqueueOutcome::SkippedCap);
    assert_eq!(
        outbox::get(&db, "d4").unwrap().unwrap().state,
        OutboxState::SkippedCap
    );
    publisher.disconnect().await;
    relay.stop().await;
}

#[tokio::test]
async fn retroactivity_enable_seeds_past_crossings_but_posts_new_ones() {
    let db = Database::open(":memory:", None).unwrap();
    let keys = keys();

    // Past evidence: gate latched (check lane) and an 8-day perfect streak.
    greek_core::sync::setting_put(&db, "alphabet_gate", "check_passed").unwrap();
    let today_day = 20_300 + 7;
    for d in 0..8 {
        let date = share_nostr::detect::days_to_iso(20_300 + d);
        let ms = share_nostr::detect::iso_to_days(&date).unwrap() * 86_400_000 + 12 * 3_600_000;
        let hlc = format!("{ms:013}-{:04}-deadbeef", 0u16);
        db.conn()
            .execute(
                "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days, source, uuid, device_id, hlc)
                 VALUES ('card-s', ?1, 4, 0.0, 0.0, 'flashcard', ?2, 'deadbeefcafe', ?3)",
                rusqlite::params![
                    format!("{date}T12:00:00Z"),
                    uuid::Uuid::new_v4().to_string(),
                    hlc
                ],
            )
            .unwrap();
    }
    let today = share_nostr::detect::days_to_iso(today_day);
    let crossings = share_nostr::detect::detect_streak_crossings(&db, &today).unwrap();
    assert_eq!(crossings.len(), 1);

    // Enable: both crossed thresholds are SEEDED, and flush posts nothing.
    let report = ops::enable(&db, &[], &today).unwrap();
    assert_eq!(report.seeded, 2);
    assert!(ops::is_enabled(&db));
    assert!(caps::already_recorded(&db, "gate.passed.check:gate").unwrap());
    assert!(caps::already_recorded(&db, &crossings[0].dedupe_key).unwrap());

    let relay = FakeRelay::start(EventScript::AcceptAll).await;
    let publisher = publisher(&keys, &relay).await;
    // Post-enable re-detection of the SAME milestones is a no-op.
    assert_eq!(
        ops::enqueue_milestone(
            &db,
            templates::SLUG_GATE_PASSED_CHECK,
            "gate.passed.check:gate",
            "gate",
            TemplateLang::En,
            None,
            None,
            &keys,
            T0,
        )
        .unwrap(),
        EnqueueOutcome::AlreadyRecorded
    );
    let r = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r, Default::default());
    assert_eq!(relay.received_count(), 0, "zero retroactive posts");

    // A NEW crossing (chapter mastered after enable) posts exactly once.
    db.conn()
        .execute(
            "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
             VALUES ('ath1-001', 31.0, 5.0, '2026-01-01T00:00:00Z', 4, 0)",
            [],
        )
        .unwrap();
    let cards = vec![greek_core::srs::Card {
        id: "ath1-001".into(),
        front: String::new(),
        back: String::new(),
        chapter: 1,
        card_type: "Vocab".into(),
        pos: "noun".into(),
        tags: vec!["athenaze-i".into()],
    }];
    let detected = share_nostr::detect::detect_chapter_mastered(&db, &cards).unwrap();
    assert_eq!(detected.len(), 1);
    let outcome = ops::enqueue_milestone(
        &db,
        detected[0].slug,
        &detected[0].dedupe_key,
        &detected[0].visible_id,
        TemplateLang::El,
        None,
        None,
        &keys,
        T0,
    )
    .unwrap();
    assert_eq!(outcome, EnqueueOutcome::Enqueued);
    let r2 = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r2.posted, 1);
    assert_eq!(relay.received_count(), 1);
    assert_eq!(
        relay.stored()[0]["content"].as_str().unwrap(),
        templates::preset(templates::SLUG_CHAPTER_MASTERED, TemplateLang::El).unwrap(),
        "EL preset on the wire"
    );
    publisher.disconnect().await;
    relay.stop().await;
}

/// Sanity path: relays that all died flush to `deferred`, not panics.
#[tokio::test]
async fn publisher_with_no_relays_errors() {
    let db = db_enabled();
    let keys = keys();
    let relay = FakeRelay::start(EventScript::AcceptAll).await;
    // Connect, then sever all relays, mimicking "relay set emptied".
    let publisher = publisher(&keys, &relay).await;
    publisher.disconnect().await;
    relay.stop().await;

    enqueue_streak7(&db, &keys, "gone:1", T0).await;
    // With every relay gone, the row defers (transient) — no panic, no
    // false delivery, no infinite hang.
    let r = ops::flush(&db, &publisher, T0).await.unwrap();
    assert_eq!(r.posted, 0);
    assert_eq!(r.deferred, 1, "relay-less publish defers: {r:?}");
    assert_eq!(
        outbox::get(&db, "gone:1").unwrap().unwrap().state,
        OutboxState::Queued
    );
}
