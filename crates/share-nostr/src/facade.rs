//! Flat integration facade — the exact surface the UniFFI bindings (and the
//! Tauri desktop bridge later) code against. All naming/policy decisions live
//! HERE so `crates/share-nostr`'s engine modules stay free of FFI conventions.

use std::sync::Mutex;

use anyhow::{anyhow, Result};
use greek_core::Database;
use greek_core::srs::Card;

use crate::identity::ShareKeys;
use crate::{detect, ops, outbox, publisher::{self, Publisher}, templates::{self, TemplateLang}};

const RELAYS_KEY: &str = "share.relays"; // CSV
const LOCALE_KEY: &str = "share.template_locale"; // en|el|auto (auto→en until OS locale hookup)

fn milestone_enabled_key(slug: &str) -> String {
    format!("share.milestone.{slug}.enabled")
}

fn setting(db: &Database, key: &str) -> Option<String> {
    db.conn()
        .query_row("SELECT value FROM user_settings WHERE key = ?1", [key], |r| r.get(0))
        .ok()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn today() -> String {
    detect::days_to_iso(now_ms() / 86_400_000)
}

fn post_lang(db: &Database) -> TemplateLang {
    match setting(db, LOCALE_KEY).as_deref() {
        Some("el") => TemplateLang::El,
        _ => TemplateLang::En, // "en" + "auto" — OS-locale plumbing is a UI follow-up
    }
}

/// Per-slug enabled flag lives in ordinary (synced) settings; default ON.
pub fn milestone_is_enabled(db: &Database, slug: &str) -> bool {
    setting(db, &milestone_enabled_key(slug)).as_deref() != Some("0")
}

// --- REQ-B1..B3: config ------------------------------------------------------

pub use crate::ops::is_enabled; // REQ-B1

pub fn relays_get(db: &Database) -> Vec<String> {
    setting(db, RELAYS_KEY)
        .map(|csv| csv.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

/// REQ-B3. None when no sync identity exists yet (enabling creates one).
pub fn share_pubkey_hex(db: &Database) -> Option<String> {
    ShareKeys::from_db(db).ok().map(|k| k.public_key_hex())
}

/// The sharing key HKDF-derives from the sync secret; enabling must ensure
/// one exists. Fresh keys land in the v4 secrets vault (never synced/exported).
fn ensure_sync_identity(db: &Database) -> Result<()> {
    match greek_core::secret_get(db, "nostr_nsec") {
        Ok(Some(_)) => Ok(()),
        _ => {
            let keys = nostr_sdk::Keys::generate();
            greek_core::secret_put(db, "nostr_nsec", &keys.secret_key().to_secret_hex())
                .map_err(|e| anyhow!("storing sync identity: {e}"))
        }
    }
}

// --- REQ-B4..B6: enable/disable/relays ---------------------------------------

pub fn enable(db: &Database, relays: &[String], cards: &[Card]) -> Result<()> {
    let relays = publisher::validate_relay_list(relays).map_err(anyhow::Error::from)?;
    if relays.is_empty() {
        return Err(anyhow!("choose at least one relay (sharing is public)"));
    }
    greek_core::sync::setting_put(db, RELAYS_KEY, &relays.join(","))?;
    ensure_sync_identity(db)?;
    let report = ops::enable(db, cards, &today()).map_err(anyhow::Error::from)?;
    if report.seeded > 0 {
        eprintln!("share: seeded {} already-crossed markers (no retroactive posts)", report.seeded);
    }
    Ok(())
}

/// Relay list edit after enable = light consent event; re-validate always.
pub fn relays_set(db: &Database, relays: &[String]) -> Result<()> {
    let relays = publisher::validate_relay_list(relays).map_err(anyhow::Error::from)?;
    greek_core::sync::setting_put(db, RELAYS_KEY, &relays.join(","))?;
    Ok(())
}

/// `forget_identity` is material no-op: derivation is deterministic from the
/// sync secret, so "forgetting" can't retire the derived pubkey (documented
/// in DESIGN-progress-sharing Appendix R). Acknowledged here for API clarity.
pub fn disable(db: &Database, _forget_identity: bool) -> Result<()> {
    ops::disable(db).map_err(anyhow::Error::from)?;
    Ok(())
}

// --- REQ-B7..B11: milestones view + templates --------------------------------

pub struct MilestoneInfo {
    pub slug: String,
    pub title_en: String,
    pub title_el: String,
    pub template_en: String,
    pub template_el: String,
    pub custom_template_en: Option<String>,
    pub custom_template_el: Option<String>,
    pub enabled: bool,
    pub last_posted_ms: Option<i64>,
    pub last_state: String,
}

fn title_for(slug: &str) -> (&'static str, &'static str) {
    match slug {
        "gate.passed.check" => ("Alphabet check passed", "Πέρασα τον έλεγχο αλφαβήτου"),
        "gate.passed.organic" => ("Mastery via practice", "Επίτευξη μέσα από εξάσκηση"),
        "chapter.mastered" => ("Chapter mastered", "Κεφάλαιο ολοκληρώθηκε"),
        "streak.7" => ("7-day streak", "7 ημέρες σερί"),
        "streak.30" => ("30-day streak", "30 ημέρες σερί"),
        "streak.100" => ("100-day streak", "100 ημέρες σερί"),
        _ => ("Milestone", "Ορόσημο"),
    }
}

fn custom_override(db: &Database, slug: &str, lang: TemplateLang) -> Option<String> {
    let key = format!("share.template.{slug}.{}", lang.code());
    setting(db, &key).filter(|v| !v.is_empty())
}

pub fn milestones(db: &Database) -> Vec<MilestoneInfo> {
    let history = outbox::history(db).unwrap_or_default();
    templates::ALL_SLUGS
        .iter()
        .map(|slug| {
            let slug = slug.to_string();
            let (title_en, title_el) = title_for(&slug);
            let latest = history.iter().find(|r| r.slug == slug);
            MilestoneInfo {
                slug: slug.clone(),
                title_en: title_en.into(),
                title_el: title_el.into(),
                template_en: templates::template_for(db, &slug, TemplateLang::En).unwrap_or_default(),
                template_el: templates::template_for(db, &slug, TemplateLang::El).unwrap_or_default(),
                custom_template_en: custom_override(db, &slug, TemplateLang::En),
                custom_template_el: custom_override(db, &slug, TemplateLang::El),
                enabled: milestone_is_enabled(db, &slug),
                last_posted_ms: latest.map(|r| r.created_ms),
                last_state: latest.map(|r| r.state.as_str().to_string()).unwrap_or_default(),
            }
        })
        .collect()
}

pub fn set_template(db: &Database, slug: &str, locale: &str, text: &str) -> Result<()> {
    let lang = if locale == "el" { TemplateLang::El } else { TemplateLang::En };
    templates::set_template_override(db, slug, lang, text).map_err(anyhow::Error::from)
}

pub fn reset_template(db: &Database, slug: &str, locale: &str) -> Result<()> {
    set_template(db, slug, locale, "") // engine: empty body = restore preset
}

pub fn set_milestone_enabled(db: &Database, slug: &str, enabled: bool) -> Result<()> {
    greek_core::sync::setting_put(db, &milestone_enabled_key(slug), if enabled { "1" } else { "0" })?;
    Ok(())
}

pub fn preview(db: &Database, slug: &str, suffix: &str) -> Result<String> {
    templates::render_content(db, slug, post_lang(db), None, Some(suffix)).map_err(anyhow::Error::from)
}

// --- REQ-B12..B14: flush + outbox views ---------------------------------------

pub async fn flush_now(db: &Mutex<Database>) -> Result<ops::FlushReport> {
    {
        let db = db.lock().unwrap();
        if !ops::is_enabled(&db) {
            return Ok(ops::FlushReport::default());
        }
    }
    // The engine's flush future holds the sqlite connection (a !Sync guard)
    // across awaits internally — legal only if it never migrates threads.
    // UniFFI runs this export on a MULTI-thread tokio runtime and requires
    // the returned future to be Send, so we pin a current-thread runtime
    // inside block_in_place: every guard stays on one OS thread, and the
    // outer future only ever awaits a plain (Send) result.
    // NOTE: block_in_place panics on a current-thread outer runtime; the
    // desktop/harness callers must use multi_thread (sync-peer does).
    tokio::task::block_in_place(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("flush runtime");
        rt.block_on(async {
            let (keys, relays) = {
                let db = db.lock().unwrap();
                let relays = relays_get(&db);
                if relays.is_empty() {
                    return Err(anyhow!("no sharing relays configured"));
                }
                (ShareKeys::from_db(&db)?, relays)
            };
            let publisher = Publisher::connect(&keys, &relays)
                .await
                .map_err(|e| anyhow!("connecting to sharing relays: {e}"))?;
            let report = {
                let db = db.lock().unwrap();
                ops::flush(&db, &publisher, now_ms()).await
            };
            publisher.disconnect().await;
            report.map_err(|e| anyhow!("flush: {e}"))
        })
    })
}

pub fn pending_count(db: &Database) -> u32 {
    outbox::pending_count(db).unwrap_or(0)
}

pub struct HistoryEntry {
    pub event_id: String,
    pub created_ms: i64,
    pub state: String,
    pub per_relay_json: String,
}

pub fn history(db: &Database, slug: &str) -> Vec<HistoryEntry> {
    outbox::history(db)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.slug == slug)
        .map(|r| {
            let event_id = serde_json::from_str::<serde_json::Value>(&r.event_json)
                .ok()
                .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_string))
                .unwrap_or_default();
            HistoryEntry {
                event_id,
                created_ms: r.created_ms,
                state: r.state.as_str().to_string(),
                per_relay_json: r.per_relay,
            }
        })
        .collect()
}

// --- REQ-B16: hooks (never fail the calling learning flow) ---------------------

/// A review just landed: invalidate the streak cache so the next lazy
/// detection pass (home stats / flush) recomputes today's standing.
pub fn mark_streak_dirty(db: &Database) {
    let _ = db
        .conn()
        .execute("DELETE FROM user_settings WHERE key = 'share.streak.cache'", []);
}

/// Gate latch just changed (check recorded). Enqueue the lane-appropriate
/// milestone; `declared_known`/vocab-history latches never enqueue. Returns
/// true when a post was queued.
pub fn detect_gate_latch(db: &Database) -> bool {
    if !ops::is_enabled(db) {
        return false;
    }
    let Some(slug) = detect::detect_gate_latch(db) else { return false };
    if !milestone_is_enabled(db, slug) {
        return false;
    }
    let Ok(keys) = ShareKeys::from_db(db) else { return false };
    let dedupe = format!("{slug}:gate");
    matches!(
        ops::enqueue_milestone(db, slug, &dedupe, "gate", post_lang(db), None, None, &keys, now_ms()),
        Ok(ops::EnqueueOutcome::Enqueued)
    )
}

/// Periodic lazy pass (home stats): due-cleared streak crossings + chapter
/// mastery. Returns how many new posts were queued.
pub fn detect_streak_chapter(db: &Database, cards: &[Card]) -> u32 {
    if !ops::is_enabled(db) {
        return 0;
    }
    let Ok(keys) = ShareKeys::from_db(db) else { return 0 };
    let mut n = 0u32;
    let t = today();
    let lang = post_lang(db);
    if let Ok(streaks) = detect::detect_streak_crossings(db, &t) {
        for st in streaks {
            if !milestone_is_enabled(db, &st.slug) {
                continue;
            }
            if matches!(
                ops::enqueue_milestone(db, st.slug, &st.dedupe_key, &st.crossed_on_day, lang,
                    Some(st.streak), None, &keys, now_ms()),
                Ok(ops::EnqueueOutcome::Enqueued)
            ) {
                n += 1;
            }
        }
    }
    if let Ok(chapters) = detect::detect_chapter_mastered(db, cards) {
        for ch in chapters {
            if !milestone_is_enabled(db, &ch.slug) {
                continue;
            }
            if matches!(
                ops::enqueue_milestone(db, ch.slug, &ch.dedupe_key, &ch.visible_id, lang,
                    None, None, &keys, now_ms()),
                Ok(ops::EnqueueOutcome::Enqueued)
            ) {
                n += 1;
            }
        }
    }
    n
}
