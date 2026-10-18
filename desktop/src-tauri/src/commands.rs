//! Frozen Tauri command contract (Agent D2 codes the UI against these
//! shapes; ALL of them return JSON strings and Err(String) so JS sees a
//! promise rejection). Bodies are plain fns over &GreekCore/&AppState so
//! they stay unit-testable without a window.

use std::collections::{HashSet, VecDeque};

use serde_json::{json, Value};
use uniffi_bindings::GreekCore;

use crate::state::{AppState, ReviewSession, SessionCard};

/// Android ReviewViewModel.SESSION_SIZE.
const SESSION_SIZE: usize = 20;

// ---------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------

/// home_stats() ->
/// '{"dueToday":0,"newToday":0,"newCap":10,"streakDays":0,
///   "practiceCorrect":0,"practiceTotal":0,"gateSatisfied":true,
///   "lettersChaptersMet":0}'
pub fn home_stats(core: &GreekCore) -> Result<String, String> {
    let stats = core.get_review_stats();
    let new_today = core.new_cards_today();
    // The daily cap is core-enforced (DEFAULT_DAILY_NEW_CAP); derive it
    // instead of hardcoding 10 so a core change propagates.
    let new_cap = new_today + core.new_cards_remaining_today();
    let practice = core.get_practice_stats();
    let gate = core.get_alphabet_gate_state();
    Ok(json!({
        "dueToday": stats.due_count,
        "newToday": new_today,
        "newCap": new_cap,
        "streakDays": stats.streak_days,
        "practiceCorrect": practice.correct,
        "practiceTotal": practice.total,
        "gateSatisfied": gate.satisfied,
        "lettersChaptersMet": gate.letters_chapters_met,
    })
    .to_string())
}

// ---------------------------------------------------------------------------
// Review session
// ---------------------------------------------------------------------------

/// Build a fresh review batch mirroring Android ReviewViewModel.loadCards /
/// fillWithNewCards: due cards first, then new cards from unlocked+available
/// chapters across every curriculum, deduplicated, capped at SESSION_SIZE.
/// The alphabet gate and daily new-cap are enforced inside GreekCore
/// (get_new_cards refuses vocab decks until the gate is met and caps the
/// per-day allowance), so this loop composes core semantics, never gates.
pub fn fill_session(core: &GreekCore) -> VecDeque<SessionCard> {
    let mut queue: VecDeque<SessionCard> = VecDeque::new();
    let mut seen: HashSet<String> = HashSet::new();

    if let Ok(due) = core.get_due_cards(SESSION_SIZE as i32) {
        for dc in due {
            if queue.len() >= SESSION_SIZE {
                break;
            }
            if seen.insert(dc.card.id.clone()) {
                queue.push_back(SessionCard {
                    card_id: dc.card.id,
                    front: dc.card.front,
                    back: dc.card.back,
                    is_new: dc.is_new,
                });
            }
        }
    }

    for cur in core.get_curricula() {
        if queue.len() >= SESSION_SIZE {
            break;
        }
        for ch in core.get_chapters(cur.textbook.clone()) {
            if queue.len() >= SESSION_SIZE {
                break;
            }
            if !ch.unlocked || !ch.available {
                continue;
            }
            let remaining = (SESSION_SIZE - queue.len()) as i32;
            for nc in core.get_new_cards(cur.textbook.clone(), ch.chapter, remaining) {
                if queue.len() >= SESSION_SIZE {
                    break;
                }
                if seen.insert(nc.id.clone()) {
                    queue.push_back(SessionCard {
                        card_id: nc.id,
                        front: nc.front,
                        back: nc.back,
                        is_new: true,
                    });
                }
            }
        }
    }
    queue
}

/// Port of Android's formatIntervalLabel: minutes below an hour, hours
/// below two days, rounded days beyond.
fn interval_label(secs: u32) -> String {
    if secs < 3600 {
        format!("{}m", (secs + 30) / 60)
    } else if secs < 172_800 {
        format!("{}h", (secs + 1800) / 3600)
    } else {
        format!("{}d", (secs + 43200) / 86400)
    }
}

/// Interval strings for Again/Hard/Good/Easy of the shown card. Falls back
/// to the same placeholder ladder Android's ContractStubs uses (10m/1h/1d/4d)
/// if FSRS previewing ever fails for the card.
fn previews_for(core: &GreekCore, card_id: &str) -> [String; 4] {
    match core.preview_intervals(card_id.to_string()) {
        Some(p) => [
            interval_label(p.again_secs),
            interval_label(p.hard_secs),
            interval_label(p.good_secs),
            interval_label(p.easy_secs),
        ],
        None => ["10m".into(), "1h".into(), "1d".into(), "4d".into()],
    }
}

/// review_current() ->
/// '{"card":{"cardId":"","front":"","back":""}|null,"remaining":0,
///   "isNew":true,"previews":["10m","1d","3d","7d"]}'
///
/// When the in-memory session is empty a fresh batch is built (mirrors the
/// Android review screen load); after a completed batch this yields
/// card:null/remaining:0 until more study material exists.
pub fn review_current(state: &AppState) -> Result<Value, String> {
    let mut session = state.session.lock().map_err(|e| e.to_string())?;
    if session.queue.is_empty() {
        session.queue = fill_session(&state.core);
    }
    Ok(review_payload(&state.core, &session))
}

fn review_payload(core: &GreekCore, session: &ReviewSession) -> Value {
    match session.queue.front() {
        Some(c) => json!({
            "card": {"cardId": c.card_id, "front": c.front, "back": c.back},
            "remaining": session.queue.len(),
            "isNew": c.is_new,
            "previews": previews_for(core, &c.card_id),
        }),
        None => json!({
            "card": Value::Null,
            "remaining": 0,
            "isNew": true,
            "previews": Vec::<String>::new(),
        }),
    }
}

/// review_rate(rating: u8) -> same JSON as review_current.
/// Submits rating 1..=4 (Again/Hard/Good/Easy) for the current card via the
/// core, pops it, and returns the next card (or the null completion shape).
/// Never rebuilds the batch — an empty queue after rating is "session done".
pub fn review_rate(state: &AppState, rating: u8) -> Result<Value, String> {
    if !(1..=4).contains(&rating) {
        return Err(format!("rating must be 1..=4, got {rating}"));
    }
    let mut session = state.session.lock().map_err(|e| e.to_string())?;
    let current = session
        .queue
        .front()
        .cloned()
        .ok_or_else(|| "no current review card (session empty)".to_string())?;
    if !state.core.submit_review(current.card_id.clone(), rating) {
        return Err(format!("submit_review failed for {}", current.card_id));
    }
    session.queue.pop_front();
    Ok(review_payload(&state.core, &session))
}

// ---------------------------------------------------------------------------
// Curriculum + dictionary
// ---------------------------------------------------------------------------

/// curriculum() ->
/// '{"gateNotMet":false,"textbooks":[{"id","title","chapters":[
///   {"index":0,"title","totalCards":0,"masteredCount":0,"unlocked":true,
///    "hint":""}]}]}'
pub fn curriculum(core: &GreekCore) -> Result<String, String> {
    let gate = core.get_alphabet_gate_state();
    let textbooks: Vec<Value> = core
        .get_curricula()
        .iter()
        .map(|c| {
            let chapters: Vec<Value> = core
                .get_chapters(c.textbook.clone())
                .iter()
                .map(|ch| {
                    json!({
                        "index": ch.chapter,
                        "title": ch.title,
                        "totalCards": ch.card_count,
                        "masteredCount": ch.mastered_count,
                        "unlocked": ch.unlocked,
                        "hint": ch.unlock_hint,
                    })
                })
                .collect();
            json!({"id": c.textbook, "title": c.display_name, "chapters": chapters})
        })
        .collect();
    Ok(json!({"gateNotMet": !gate.satisfied, "textbooks": textbooks}).to_string())
}

/// dict_search(q) -> '[{"lemma","pos","gloss"}]' (max 25). Without a
/// morphology lexicon installed the core's search returns [], which is the
/// documented graceful degradation.
pub fn dict_search(core: &GreekCore, q: &str) -> Result<String, String> {
    let results: Vec<Value> = core
        .search_dictionary(q.to_string(), 25)
        .iter()
        .map(|r| json!({"lemma": r.headword, "pos": r.pos, "gloss": r.definition}))
        .collect();
    Ok(Value::Array(results).to_string())
}

// ---------------------------------------------------------------------------
// Device sync (all core work happens inside GreekCore's sync_* exports;
// these wrappers only shape JSON and sequence transports)
// ---------------------------------------------------------------------------

/// sync_status() ->
/// '{"deviceId":"","irohConfigured":false,"nostrConfigured":false,
///   "relays":"csv","paired":[{"deviceId","label","lastSyncHlc"}]}'
pub fn sync_status(state: &AppState) -> Result<String, String> {
    let core = &state.core;
    let nostr = core.sync_nostr_config();
    let paired: Vec<Value> = core
        .sync_paired_devices()
        .iter()
        .map(|p| {
            json!({"deviceId": p.device_id, "label": p.label, "lastSyncHlc": p.last_sync_hlc})
        })
        .collect();
    Ok(json!({
        "deviceId": core.sync_device_id(),
        "irohConfigured": state.iroh_configured(),
        "nostrConfigured": nostr.has_identity,
        "relays": nostr.relays_csv,
        "paired": paired,
    })
    .to_string())
}

/// sync_ticket() -> '"greeksync://ticket/..."' (async; JSON string of the
/// ticket URL). Creating the ticket lazily opens the iroh node, which also
/// flips sync_status().irohConfigured to true.
pub async fn sync_ticket(state: &AppState) -> Result<String, String> {
    let ticket = state
        .core
        .sync_iroh_ticket()
        .await
        .map_err(|e| e.to_string())?;
    Ok(Value::String(ticket).to_string())
}

/// sync_join(ticket) -> '"ok"'. Accepts a raw ticket or the greeksync:// URL
/// (the core normalizes both).
pub async fn sync_join(state: &AppState, ticket: &str) -> Result<String, String> {
    state
        .core
        .sync_iroh_join(ticket.to_string())
        .await
        .map_err(|e| e.to_string())?;
    Ok(Value::String("ok".to_string()).to_string())
}

/// sync_now() ->
/// '{"reviewsSent":0,"reviewsReceived":0,"settingsApplied":0,"viaIroh":false,
///   "viaNostr":false}'
///
/// Transport order: one iroh round first (push + peer sync + pull + merge),
/// then nostr pull+push. Transports that are not configured are skipped;
/// the core skips publishing empty nostr deltas internally. Failing
/// transports are tolerated while at least one succeeds; if every attempted
/// transport fails the combined error is returned.
pub async fn sync_now(state: &AppState) -> Result<String, String> {
    let core = &state.core;
    let iroh_on = state.iroh_configured();
    let nostr_on = core.sync_nostr_config().has_identity;
    if !iroh_on && !nostr_on {
        return Err("no sync transport configured: pair a device first \
                    (sync_ticket / sync_join) or run nostr_identity_ensure"
            .to_string());
    }

    let mut reviews_sent = 0u32;
    let mut reviews_received = 0u32;
    let mut settings_applied = 0u32;
    let mut via_iroh = false;
    let mut via_nostr = false;
    let mut errors: Vec<String> = Vec::new();

    if iroh_on {
        match core.sync_iroh_now().await {
            Ok(s) => {
                reviews_sent += s.reviews_sent;
                reviews_received += s.reviews_received;
                settings_applied += s.settings_applied;
                via_iroh = true;
            }
            Err(e) => errors.push(format!("iroh: {e}")),
        }
    }

    if nostr_on {
        match core.sync_nostr_pull().await {
            Ok(s) => {
                reviews_received += s.reviews_received;
                settings_applied += s.settings_applied;
                via_nostr = true;
            }
            Err(e) => errors.push(format!("nostr pull: {e}")),
        }
        match core.sync_nostr_push().await {
            Ok(s) => {
                reviews_sent += s.reviews_sent;
                via_nostr = true;
            }
            Err(e) => errors.push(format!("nostr push: {e}")),
        }
    }

    if !via_iroh && !via_nostr {
        return Err(format!("all configured sync transports failed: {}", errors.join("; ")));
    }

    Ok(json!({
        "reviewsSent": reviews_sent,
        "reviewsReceived": reviews_received,
        "settingsApplied": settings_applied,
        "viaIroh": via_iroh,
        "viaNostr": via_nostr,
    })
    .to_string())
}

/// nostr_identity_ensure() -> '"<pubkey hex>"'. Generates and persists an
/// identity when missing; when one already exists, derives the pubkey from
/// the stored secret. GreekCore exposes identity generation but no pubkey
/// getter, so the derivation reads `user_settings.nostr_nsec` from the app
/// database read-only and runs the same Keys::parse the core itself uses.
/// (If the core grows a pubkey getter, switch to it and drop this peek.)
pub fn nostr_identity_ensure(state: &AppState) -> Result<String, String> {
    if !state.core.sync_nostr_config().has_identity {
        let pubkey = state.core.sync_generate_nostr_identity();
        return Ok(Value::String(pubkey).to_string());
    }
    let conn = rusqlite::Connection::open_with_flags(
        &state.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| format!("open db for pubkey derivation: {e}"))?;
    let nsec: String = conn
        .query_row(
            "SELECT value FROM user_settings WHERE key = 'nostr_nsec'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("read stored nostr identity: {e}"))?;
    let keys = nostr_sdk::prelude::Keys::parse(nsec.trim())
        .map_err(|e| format!("stored nostr identity is invalid: {e}"))?;
    Ok(Value::String(keys.public_key().to_hex()).to_string())
}

/// nostr_relays_set(csv) -> '"ok"'. Persists the CSV relay list through the
/// core so it also propagates via settings sync.
pub fn nostr_relays_set(core: &GreekCore, csv: &str) -> Result<String, String> {
    core.sync_set_nostr_relays(csv.to_string());
    Ok(Value::String("ok".to_string()).to_string())
}

/// backup_export() -> the full SyncBatch JSON (NOT double-encoded: the
/// command's return value IS the batch JSON text — JS does JSON.parse once).
pub fn backup_export(core: &GreekCore) -> Result<String, String> {
    Ok(core.sync_export_backup_json())
}

/// backup_import(json) -> '{"reviewsReceived":0,"settingsApplied":0}'.
/// Strict v1 wire version enforced inside the core.
pub fn backup_import(core: &GreekCore, json_text: &str) -> Result<String, String> {
    let stats = core
        .sync_import_backup_json(json_text.to_string())
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "reviewsReceived": stats.reviews_received,
        "settingsApplied": stats.settings_applied,
    })
    .to_string())
}
