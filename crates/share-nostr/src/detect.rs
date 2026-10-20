//! Pure milestone detectors over `greek_core` state. Each returns only the
//! facts; the caller (`ops`) decides whether to seed (enable-time
//! zero-retroactivity) or enqueue (post-enable crossings).
//!
//! The streak detector is the subtle one: day D *counts* iff (a) at least
//! one `review_logs` row lands on D and (b) NO card had `due_date ≤` end-of-D
//! left unreviewed at end-of-D. Due dates for past days are reconstructed by
//! FSRS **state replay cut at end-of-day** — the same fold shape as
//! `greek_core::sync::rebuild_memory_states` (whole-day elapsed gaps,
//! retention 0.9, rating outside 1–4 treated as Good, due = ts + interval),
//! replayed here because core's day-math helpers are `pub(crate)`.

use crate::errors::{Result, ShareError};
use crate::templates;

/// Streak thresholds that constitute milestones.
pub const STREAK_THRESHOLDS: [u32; 3] = [7, 30, 100];

// --- UTC day math (mirrors greek_core::srs private helpers) ----------------

fn ymd_to_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let m = if m <= 2 { m + 9 } else { m - 3 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Parse `YYYY-MM-DD` (or an ISO8601 timestamp whose date part is that) into
/// epoch days.
pub fn iso_to_days(iso: &str) -> Option<i64> {
    if iso.len() < 10 {
        return None;
    }
    let year: i64 = iso[0..4].parse().ok()?;
    let month: i64 = iso[5..7].parse().ok()?;
    let day: i64 = iso[8..10].parse().ok()?;
    Some(ymd_to_days(year, month, day))
}

/// Format epoch days as `YYYY-MM-DD`.
pub fn days_to_iso(days: i64) -> String {
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn iso_to_secs(iso: &str) -> Option<i64> {
    let days = iso_to_days(iso)?;
    let hours: i64 = iso.get(11..13).and_then(|s| s.parse().ok()).unwrap_or(0);
    let minutes: i64 = iso.get(14..16).and_then(|s| s.parse().ok()).unwrap_or(0);
    let seconds: i64 = iso.get(17..19).and_then(|s| s.parse().ok()).unwrap_or(0);
    Some(days * 86400 + hours * 3600 + minutes * 60 + seconds)
}

/// Whole-day elapsed between two ISO timestamps (saturating, date-part
/// granularity) — same rule as core's fold.
fn days_between(from: &str, to: &str) -> u32 {
    let f = iso_to_days(from).unwrap_or(0);
    let t = iso_to_days(to).unwrap_or(0);
    t.saturating_sub(f).max(0) as u32
}

// --- FSRS replay -----------------------------------------------------------

/// One card's review timeline: `(event_ts_secs, due_secs_after_event)`,
/// ordered chronologically by the fold's total order.
struct CardTimeline {
    /// Per event: (event ts secs, due ts secs computed from that event).
    steps: Vec<(i64, i64)>,
}

/// Replay `review_logs` exactly as core's fold does, but keep the full
/// (ts → due) timeline per card instead of collapsing to the latest state.
fn card_timelines(db: &greek_core::Database) -> Result<Vec<CardTimeline>> {
    const DESIRED_RETENTION: f32 = 0.9;
    let fsrs = fsrs::FSRS::new(Some(&[]))
        .map_err(|e| ShareError::Nostr(format!("FSRS init failed: {e}")))?;

    // Same total order as greek_core::sync::rebuild_memory_states:
    // (hlc, uuid, id) within a card.
    let mut per_card: std::collections::BTreeMap<String, Vec<(String, i64)>> =
        std::collections::BTreeMap::new();
    {
        let mut stmt = db.conn().prepare(
            "SELECT card_id, timestamp, rating, COALESCE(hlc, ''), COALESCE(uuid, ''), id
             FROM review_logs
             ORDER BY card_id, COALESCE(hlc, timestamp), COALESCE(uuid, ''), id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (card, ts, rating) = row?;
            per_card.entry(card).or_default().push((ts, rating));
        }
    }

    let mut timelines = Vec::with_capacity(per_card.len());
    for (_card, events) in per_card {
        let mut timeline = CardTimeline { steps: vec![] };
        let mut stability: f32 = 0.0;
        let mut difficulty: f32 = 0.0;
        let mut last_review: Option<String> = None;
        for (ts, rating) in events {
            let elapsed = last_review
                .as_deref()
                .map(|prev| days_between(prev, &ts))
                .unwrap_or(0);
            let memory = last_review
                .as_ref()
                .map(|_| fsrs::MemoryState { stability, difficulty });
            let next = fsrs
                .next_states(memory, DESIRED_RETENTION, elapsed)
                .map_err(|e| ShareError::Nostr(format!("FSRS replay failed: {e}")))?;
            // Mirror core's fold exactly: rating outside 1–4 = Good.
            let chosen = match rating {
                1 => &next.again,
                2 => &next.hard,
                3 => &next.good,
                4 => &next.easy,
                _ => &next.good,
            };
            stability = chosen.memory.stability;
            difficulty = chosen.memory.difficulty;
            let ts_secs = iso_to_secs(&ts)
                .ok_or_else(|| ShareError::Nostr(format!("bad review timestamp {ts:?}")))?;
            let due_secs = ts_secs + (chosen.interval as f64 * 86400.0).round().max(0.0) as i64;
            timeline.steps.push((ts_secs, due_secs));
            last_review = Some(ts);
        }
        timelines.push(timeline);
    }
    Ok(timelines)
}

// --- Streak cache ----------------------------------------------------------

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StreakCache {
    /// UTC day the streak value applies to ("YYYY-MM-DD").
    as_of: String,
    /// review_logs row count used as the freshness stamp.
    n_reviews: u64,
    /// Consecutive due-cleared days ending on `as_of`.
    streak: u32,
}

const STREAK_CACHE_KEY: &str = "share.streak.cache";

fn read_streak_cache(db: &greek_core::Database, today: &str) -> Option<u32> {
    let n: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM review_logs", [], |r| r.get(0))
        .ok()?;
    let raw: String = db
        .conn()
        .query_row(
            "SELECT value FROM user_settings WHERE key = ?1",
            [STREAK_CACHE_KEY],
            |r| r.get(0),
        )
        .ok()?;
    let cache: StreakCache = serde_json::from_str(&raw).ok()?;
    if cache.as_of == today && cache.n_reviews == n as u64 {
        Some(cache.streak)
    } else {
        None
    }
}

/// Write the cache STRAIGHT into `user_settings` (never through
/// `setting_put`): the cache is local, ephemeral compute state and must not
/// sync to other devices.
fn write_streak_cache(
    db: &greek_core::Database,
    today: &str,
    streak: u32,
) -> Result<()> {
    let n: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM review_logs", [], |r| r.get(0))?;
    let cache = StreakCache {
        as_of: today.to_string(),
        n_reviews: n as u64,
        streak,
    };
    db.conn().execute(
        "INSERT INTO user_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![STREAK_CACHE_KEY, serde_json::to_string(&cache)?],
    )?;
    Ok(())
}

/// Compute the due-cleared streak ending at `today` ("YYYY-MM-DD"): the
/// count of consecutive days back from `today` on which the learner
/// reviewed at least once AND had no backlog at the relevant cutoff:
///
/// - past day D: counts iff ≥1 review on D and no card's latest replayed
///   state at end-of-D had `due <` start-of-(D+1) (no card lay due-and-
///   unreviewed when D ended);
/// - today: counts iff ≥1 review today and no card lay overdue already at
///   start-of-today (cards becoming due *during* today don't disqualify it;
///   a backlog stale since before today does);
/// - a `today` with no reviews yet doesn't fail the run — the scan steps
///   back to yesterday instead of zeroing the streak mid-day.
///
/// Result cached per (db, day) in the local-only `share.streak.cache` json.
pub fn current_streak(db: &greek_core::Database, today: &str) -> Result<u32> {
    if let Some(cached) = read_streak_cache(db, today) {
        return Ok(cached);
    }
    let today_day = iso_to_days(today)
        .ok_or_else(|| ShareError::Nostr(format!("bad date {today:?}")))?;

    let timelines = card_timelines(db)?;
    let first_day: Option<i64> = timelines
        .iter()
        .filter_map(|t| t.steps.first().map(|(ts, _)| ts.div_euclid(86400)))
        .min();

    let mut review_days: std::collections::BTreeSet<i64> =
        std::collections::BTreeSet::new();
    for t in &timelines {
        for (ts, _) in &t.steps {
            review_days.insert(ts.div_euclid(86400));
        }
    }

    let qualifies = |day: i64| -> bool {
        if !review_days.contains(&day) {
            return false;
        }
        let event_cutoff = (day + 1) * 86400; // first second NOT in `day`
        let backlog_cutoff = if day == today_day {
            day * 86400 // start of today: only a stale backlog violates
        } else {
            event_cutoff // end of a past day
        };
        !timelines.iter().any(|t| {
            let mut lo = 0usize;
            let mut hi = t.steps.len();
            while lo < hi {
                let mid = (lo + hi) / 2;
                if t.steps[mid].0 < event_cutoff {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            match lo.checked_sub(1) {
                Some(idx) => t.steps[idx].1 < backlog_cutoff,
                None => false, // card didn't exist yet on `day`
            }
        })
    };

    let streak = match first_day {
        None => 0,
        Some(first) => {
            let mut streak = 0u32;
            let mut day = today_day;
            while day >= first {
                if qualifies(day) {
                    streak += 1;
                    day -= 1;
                } else if day == today_day && !review_days.contains(&today_day) {
                    day -= 1; // today hasn't started studying; check yesterday
                } else {
                    break;
                }
            }
            streak
        }
    };

    write_streak_cache(db, today, streak)?;
    Ok(streak)
}

// --- Public detectors --------------------------------------------------------

/// The gate-latch slug for the persisted `alphabet_gate` method, honoring
/// lane: `check_passed` and `organic` are measured evidence and post;
/// `declared_known` NEVER posts (self-declaration is not evidence);
/// `vocab_history` is grandfathered behavior, similarly not evidence.
pub fn detect_gate_latch(db: &greek_core::Database) -> Option<&'static str> {
    let method: String = db
        .conn()
        .query_row(
            "SELECT value FROM user_settings WHERE key = 'alphabet_gate'",
            [],
            |row| row.get(0),
        )
        .ok()?;
    match method.as_str() {
        "check_passed" => Some(templates::SLUG_GATE_PASSED_CHECK),
        "organic" => Some(templates::SLUG_GATE_PASSED_ORGANIC),
        _ => None, // declared_known / vocab_history / unknown
    }
}

/// A ready-to-enqueue chapter-mastered milestone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterMilestone {
    pub slug: &'static str,
    pub dedupe_key: String,
    pub visible_id: String,
}

/// Chapters where `mastered_count == card_count` (same mastered definition
/// the curriculum view uses: [u]stability ≥ 30d for vocab, reps ≥ 2 for
/// letters). Excludes empty/unavailable chapters.
pub fn detect_chapter_mastered(
    db: &greek_core::Database,
    cards: &[greek_core::srs::Card],
) -> Result<Vec<ChapterMilestone>> {
    let mut milestones = Vec::new();
    for textbook_info in greek_core::curriculum::Curriculum::get_textbooks() {
        for ch in greek_core::curriculum::Curriculum::get_chapters(
            db,
            &textbook_info.textbook,
            cards,
        ) {
            if ch.available && ch.card_count > 0 && ch.mastered_count >= ch.card_count {
                let id = format!("{}/{}", textbook_info.textbook, ch.chapter);
                milestones.push(ChapterMilestone {
                    slug: templates::SLUG_CHAPTER_MASTERED,
                    dedupe_key: format!("{}:{}", templates::SLUG_CHAPTER_MASTERED, id),
                    visible_id: id,
                });
            }
        }
    }
    Ok(milestones)
}

/// A ready-to-enqueue streak milestone. `crossed_on_day` is the UTC date
/// when the current run first reached the threshold (today − (streak − T)),
/// so thresholds crossed by an un-witnessed jump (6→8) still dedupe
/// deterministically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreakMilestone {
    pub threshold: u32,
    pub slug: &'static str,
    pub dedupe_key: String,
    pub crossed_on_day: String,
    pub streak: u32,
}

/// Every streak threshold the current run has crossed.
pub fn detect_streak_crossings(
    db: &greek_core::Database,
    today: &str,
) -> Result<Vec<StreakMilestone>> {
    let streak = current_streak(db, today)?;
    let today_day = iso_to_days(today)
        .ok_or_else(|| ShareError::Nostr(format!("bad date {today:?}")))?;
    let mut out = Vec::new();
    for t in STREAK_THRESHOLDS {
        if streak >= t {
            let crossed = days_to_iso(today_day - i64::from(streak - t));
            let slug = match t {
                7 => templates::SLUG_STREAK_7,
                30 => templates::SLUG_STREAK_30,
                _ => templates::SLUG_STREAK_100,
            };
            out.push(StreakMilestone {
                threshold: t,
                slug,
                dedupe_key: format!("{slug}:{crossed}"),
                crossed_on_day: crossed,
                streak,
            });
        }
    }
    Ok(out)
}

/// Convenience single-value detector (kept for parity with the design
/// naming; equivalent to `current_streak` → `None` when 0).
pub fn detect_due_cleared_streak(
    db: &greek_core::Database,
    today: &str,
) -> Result<Option<u32>> {
    let s = current_streak(db, today)?;
    Ok((s > 0).then_some(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use greek_core::Database;

    /// Insert a review row directly (submit_review is "now"-only; detectors
    /// need backdated rows).
    fn seed_review(db: &Database, card: &str, iso: &str, rating: i64) {
        let ms = iso_to_secs(iso).unwrap() * 1000;
        let hlc = format!("{ms:013}-{:04}-deadbeef", 0u16);
        db.conn()
            .execute(
                "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days, source, uuid, device_id, hlc)
                 VALUES (?1, ?2, ?3, 0.0, 0.0, 'flashcard', ?4, 'deadbeefcafe', ?5)",
                rusqlite::params![card, iso, rating, uuid::Uuid::new_v4().to_string(), hlc],
            )
            .unwrap();
    }

    #[test]
    fn gate_latch_lane_mapping() {
        let db = Database::open(":memory:", None).unwrap();
        assert_eq!(detect_gate_latch(&db), None);
        for (latch, expected) in [
            ("check_passed", Some(templates::SLUG_GATE_PASSED_CHECK)),
            ("organic", Some(templates::SLUG_GATE_PASSED_ORGANIC)),
            ("declared_known", None),
            ("vocab_history", None),
            ("garbage", None),
        ] {
            let fresh = Database::open(":memory:", None).unwrap();
            fresh
                .conn()
                .execute(
                    "INSERT INTO user_settings (key, value) VALUES ('alphabet_gate', ?1)",
                    [latch],
                )
                .unwrap();
            assert_eq!(detect_gate_latch(&fresh), expected, "latch {latch}");
        }
    }

    #[test]
    fn streak_scenario_perfect_run_counts_every_day() {
        let db = Database::open(":memory:", None).unwrap();
        // One card, reviewed every day at noon for 10 days, rating Easy:
        // intervals always exceed the gap to the next review, so the card is
        // never due-and-unreviewed at an end-of-day.
        for d in 0..10 {
            let date = days_to_iso(20_000 + d);
            seed_review(&db, "card-a", &format!("{date}T12:00:00Z"), 4);
        }
        let today = days_to_iso(20_009);
        assert_eq!(current_streak(&db, &today).unwrap(), 10);
        // One day before: 9.
        assert_eq!(current_streak(&db, &days_to_iso(20_008)).unwrap(), 9);
        // And a day with NO review at all (asking about "tomorrow") keeps
        // yesterday's run intact-ish: 0 days counted today but the run
        // ending yesterday is ... by our rule a review-free today doesn't
        // fail the run.
        assert_eq!(current_streak(&db, &days_to_iso(20_010)).unwrap(), 10);
    }

    #[test]
    fn streak_scenario_missed_day_breaks_run() {
        let db = Database::open(":memory:", None).unwrap();
        // Days 0..=4 and 6..=9 reviewed (day 5 skipped wholly).
        for d in (0..=4).chain(6..=9) {
            let date = days_to_iso(20_100 + d);
            seed_review(&db, "card-b", &format!("{date}T12:00:00Z"), 4);
        }
        // Ending at day 9: four consecutive counting days (6,7,8,9).
        assert_eq!(current_streak(&db, &days_to_iso(20_109)).unwrap(), 4);
        // Ending at day 4 the run was 5.
        assert_eq!(current_streak(&db, &days_to_iso(20_104)).unwrap(), 5);
    }

    #[test]
    fn streak_scenario_overdue_card_disqualifies_despite_reviews() {
        let db = Database::open(":memory:", None).unwrap();
        // Card C reviewed once (Good → first interval ~2.9 days). Cards X_i
        // reviewed daily thereafter so every day HAS >= 1 review, but card C
        // is due-and-unreviewed from its due date onward.
        let d0 = 20_200;
        seed_review(&db, "card-c", &format!("{}T00:30:00Z", days_to_iso(d0)), 3);
        for i in 1..=8 {
            let date = days_to_iso(d0 + i);
            seed_review(&db, &format!("card-x{i}"), &format!("{date}T12:00:00Z"), 4);
        }
        // Days 0,1,2: clean (card C's due sits around day 2.9̇+1 = within
        // day3 or beyond). Day 3 onward: card C overdue → days DO NOT count.
        // Assert the day-0..2 behavior via streak-at-day values:
        assert!(current_streak(&db, &days_to_iso(d0)).unwrap() >= 1);
        // Ending at day 8: card C is unreviewed and long overdue
        // with reviews happening → streak must be 0.
        assert_eq!(current_streak(&db, &days_to_iso(d0 + 8)).unwrap(), 0);
        // ... unless card C finally gets reviewed: day 9 heals it.
        seed_review(&db, "card-c", &format!("{}T12:00:00Z", days_to_iso(d0 + 9)), 4);
        let streak = current_streak(&db, &days_to_iso(d0 + 9)).unwrap();
        // card-x8 hasn't been re-reviewed since day 8: its Easy interval
        // (~10d) keeps it from being overdue on day 9, so day 9 counts.
        assert_eq!(streak, 1, "healing review makes day 9 count, run = 1");
    }

    #[test]
    fn streak_cache_invalidates_on_new_reviews_and_stays_off_sync() {
        let db = Database::open(":memory:", None).unwrap();
        seed_review(&db, "card-z", "2026-10-01T12:00:00Z", 4);
        let a = current_streak(&db, "2026-10-01").unwrap();
        assert_eq!(a, 1);
        // Cache lives in user_settings but NOT settings_sync.
        let in_sync: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM settings_sync WHERE key = 'share.streak.cache'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(in_sync, 0, "streak cache must never sync");
        // Cached read agrees.
        assert_eq!(current_streak(&db, "2026-10-01").unwrap(), 1);
        // New review invalidates.
        seed_review(&db, "card-z", "2026-10-02T12:00:00Z", 4);
        assert_eq!(current_streak(&db, "2026-10-02").unwrap(), 2);
    }

    #[test]
    fn streak_crossings_dedupe_deterministically_on_jumps() {
        let db = Database::open(":memory:", None).unwrap();
        // 8 consecutive clean days — crosses the 7-day threshold "yesterday".
        for d in 0..8 {
            let date = days_to_iso(20_300 + d);
            seed_review(&db, "card-s", &format!("{date}T12:00:00Z"), 4);
        }
        let crossings =
            detect_streak_crossings(&db, &days_to_iso(20_307)).unwrap();
        assert_eq!(crossings.len(), 1);
        let c = &crossings[0];
        assert_eq!(c.slug, templates::SLUG_STREAK_7);
        // The 7th day of the run was day 6 (0-indexed) of the 8-day run.
        assert_eq!(c.crossed_on_day, days_to_iso(20_306));
        assert_eq!(c.dedupe_key, format!("streak.7:{}", days_to_iso(20_306)));
        // A later call on day 9 (9-day run) yields the SAME dedupe key.
        seed_review(&db, "card-s", &format!("{}T12:00:00Z", days_to_iso(20_308)), 4);
        let later = detect_streak_crossings(&db, &days_to_iso(20_308)).unwrap();
        assert_eq!(later[0].dedupe_key, c.dedupe_key, "crossing keys are run-stable");
    }

    #[test]
    fn chapter_mastered_uses_the_curriculum_definition() {
        let db = Database::open(":memory:", None).unwrap();
        let card = greek_core::srs::Card {
            id: "ath1-001".into(),
            front: String::new(),
            back: String::new(),
            chapter: 1,
            card_type: "Vocab".into(),
            pos: "noun".into(),
            tags: vec!["athenaze-i".into()],
        };
        // Not mastered: no memory state.
        assert!(detect_chapter_mastered(&db, &[card.clone()]).unwrap().is_empty());
        // Low-stability state: still not mastered (< 30d).
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-001', 5.0, 5.0, '2026-01-01T00:00:00Z', 3, 0)",
                [],
            )
            .unwrap();
        assert!(detect_chapter_mastered(&db, &[card.clone()]).unwrap().is_empty());
        // Stability 30d: mastered_count == card_count == 1 → milestone.
        db.conn()
            .execute("UPDATE memory_states SET stability = 30.0 WHERE card_id = 'ath1-001'", [])
            .unwrap();
        let ms = detect_chapter_mastered(&db, &[card]).unwrap();
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].slug, templates::SLUG_CHAPTER_MASTERED);
        assert_eq!(ms[0].dedupe_key, "chapter.mastered:athenaze/1");
        assert_eq!(ms[0].visible_id, "athenaze/1");
    }
}
