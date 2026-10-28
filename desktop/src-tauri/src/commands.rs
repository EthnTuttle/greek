//! Frozen Tauri command contract (Agent D2 codes the UI against these
//! shapes; ALL of them return JSON strings and Err(String) so JS sees a
//! promise rejection). Bodies are plain fns over &GreekCore/&AppState so
//! they stay unit-testable without a window.

use std::collections::VecDeque;

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

/// Build a fresh review batch via the core's one-pass builder
/// (GreekCore::build_review_session — the C3 fix): due cards first (never
/// gated), then new cards across curricula consuming a SINGLE daily-budget
/// snapshot, exactly like Android ReviewViewModel. The alphabet gate and
/// daily new-cap are enforced inside GreekCore, so this composes core
/// semantics, never gates.
pub fn fill_session(core: &GreekCore) -> VecDeque<SessionCard> {
    match core.build_review_session(SESSION_SIZE as u32) {
        Ok(cards) => cards
            .into_iter()
            .map(|c| SessionCard {
                card_id: c.card.id,
                front: c.card.front,
                back: c.card.back,
                is_new: c.is_new,
            })
            .collect(),
        Err(_) => VecDeque::new(),
    }
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

/// D8: drop the cached in-memory review batch. A newly imported / synced
/// review from another device can retire (or reschedule) a card this device
/// is still holding as "new"; rebuilding on the next review_current is the
/// only safe read because the batch is a point-in-time snapshot.
pub fn invalidate_session(state: &AppState) {
    if let Ok(mut session) = state.session.lock() {
        session.queue.clear();
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

// ---------------------------------------------------------------------------
// Alphabet check + chapter unlock (D1 — mirror Android's PracticeViewModel
// check mode and CurriculumViewModel tap flows; ALL gating stays in the core)
// ---------------------------------------------------------------------------

/// alphabet_check_generate() ->
/// '[{"cardId","glyphToName":true,"prompt","choices":["",…],"correctIndex":0}]'
///
/// Mirrors Android PracticeViewModel.loadQuestions in check mode: one call
/// yields the full 24-question stratified batch (fresh shuffle per attempt).
/// correctIndex is shipped to the client for local scoring, exactly as
/// Android's McqData carries it.
pub fn alphabet_check_generate(core: &GreekCore) -> Result<String, String> {
    let questions: Vec<Value> = core
        .generate_alphabet_check()
        .iter()
        .map(|q| {
            json!({
                "cardId": q.card_id,
                "glyphToName": q.glyph_to_name,
                "prompt": q.prompt,
                "choices": q.choices,
                "correctIndex": q.correct_index,
            })
        })
        .collect();
    Ok(Value::Array(questions).to_string())
}

/// alphabet_check_answer(cardId, correct) -> '"ok"'.
/// Per-question report, mirroring Android selectChoice → submitPracticeResult:
/// MCQ answers never earn the accent-exact bonus, so exact=false (a miss
/// becomes an FSRS-Again item, a hit a Good review).
pub fn alphabet_check_answer(
    core: &GreekCore,
    card_id: &str,
    correct: bool,
) -> Result<String, String> {
    if core.submit_practice_result(card_id.to_string(), correct, false) {
        Ok(Value::String("ok".to_string()).to_string())
    } else {
        Err(format!("submit_practice_result failed for {card_id}"))
    }
}

/// alphabet_check_verdict(correct, total) -> '{"gateSatisfied":false}'.
/// Mirrors Android recordVerdict: the score is computed client-side, the
/// verdict is persisted here (>=22 of >=24 latches the gate as check_passed
/// inside the core — desktop adds no policy of its own).
pub fn alphabet_check_verdict(
    core: &GreekCore,
    correct: u32,
    total: u32,
) -> Result<String, String> {
    let satisfied = core.record_alphabet_check_result(correct, total);
    Ok(json!({"gateSatisfied": satisfied}).to_string())
}

/// chapter_unlock(textbook, chapter) -> '{"unlocked":false,"hint":""}'.
/// Mirrors Android CurriculumViewModel.startChapter → core.unlockChapter:
/// the core itself enforces the alphabet gate (vocabulary-track unlocks are
/// refused while the gate is unmet) and records the unlock. We only report
/// the post-attempt state plus the core-supplied hint for the row.
pub fn chapter_unlock(core: &GreekCore, textbook: &str, chapter: i32) -> Result<String, String> {
    core.unlock_chapter(textbook.to_string(), chapter);
    let view = core
        .get_chapters(textbook.to_string())
        .into_iter()
        .find(|c| c.chapter == chapter);
    Ok(json!({
        "unlocked": view.as_ref().map(|c| c.unlocked).unwrap_or(false),
        "hint": view.map(|c| c.unlock_hint).unwrap_or_default(),
    })
    .to_string())
}

/// dict_search(q) -> '[{"lemma","pos","gloss"}]' (max 25). Without a
/// morphology lexicon installed the core's search returns [], which is the
/// documented graceful degradation; pair with dict_status() to tell
/// "not installed" apart from "no matches".
pub fn dict_search(core: &GreekCore, q: &str) -> Result<String, String> {
    let results: Vec<Value> = core
        .search_dictionary(q.to_string(), 25)
        .iter()
        .map(|r| json!({"lemma": r.headword, "pos": r.pos, "gloss": r.definition}))
        .collect();
    Ok(Value::Array(results).to_string())
}

/// dict_status() -> '{"installed":false,"path":null}'.
///
/// D2: fresh installs of the bundled app used to have NO morphology.sqlite,
/// making every dictionary lookup look like a plain "No matches". The UI
/// calls this once when the dictionary tab opens so it can show a real
/// "Dictionary not installed" message instead. `installed` means a
/// non-empty lexicon was resolved (env override → bundled resource → data
/// dir) and attached at init; `path` is the resolved location when present.
pub fn dict_status(state: &AppState) -> Result<String, String> {
    Ok(json!({
        "installed": state.dictionary_installed(),
        "path": state
            .morph_path
            .as_ref()
            .map(|p| Value::String(p.to_string_lossy().into_owned()))
            .unwrap_or(Value::Null),
    })
    .to_string())
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
///   "viaNostr":false,"errors":[],"transports":{...}}'
///
/// Transport order: one iroh round first (push + peer sync + pull + merge),
/// then nostr pull+push. Transports that are not configured are skipped;
/// the core skips publishing empty nostr deltas internally.
///
/// Outcome is reported PER OPERATION, not just aggregated: each attempted
/// transport gets a `transports.<name>` entry (`iroh: {"ok":bool,
/// "error":...}`, `nostr: {"pushOk":..,"pullOk":..,"pushError","pullError"}`)
/// and any failure is also flattened into top-level `errors`. So a failed
/// pull with a successful (even empty) push is NEVER a clean success —
/// `errors` is non-empty and `nostr.pullOk` is false. An error is returned
/// only when every attempted operation failed.
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
    let mut transports = serde_json::Map::new();

    if iroh_on {
        match core.sync_iroh_now().await {
            Ok(s) => {
                reviews_sent += s.reviews_sent;
                reviews_received += s.reviews_received;
                settings_applied += s.settings_applied;
                via_iroh = true;
                transports.insert("iroh".into(), json!({"ok": true}));
            }
            Err(e) => {
                let msg = format!("iroh: {e}");
                transports.insert(
                    "iroh".into(),
                    json!({"ok": false, "error": msg.clone()}),
                );
                errors.push(msg);
            }
        }
    }

    if nostr_on {
        let pull = core.sync_nostr_pull().await;
        let push = core.sync_nostr_push().await;
        let (pull_ok, pull_err) = match pull {
            Ok(s) => {
                reviews_received += s.reviews_received;
                settings_applied += s.settings_applied;
                (true, Value::Null)
            }
            Err(e) => {
                let msg = format!("nostr pull: {e}");
                errors.push(msg.clone());
                (false, Value::String(msg))
            }
        };
        let (push_ok, push_err) = match push {
            Ok(s) => {
                reviews_sent += s.reviews_sent;
                (true, Value::Null)
            }
            Err(e) => {
                let msg = format!("nostr push: {e}");
                errors.push(msg.clone());
                (false, Value::String(msg))
            }
        };
        via_nostr = push_ok || pull_ok;
        transports.insert(
            "nostr".into(),
            json!({
                "pushOk": push_ok,
                "pullOk": pull_ok,
                "pushError": push_err,
                "pullError": pull_err,
            }),
        );
    }

    if !via_iroh && !via_nostr {
        return Err(format!("all configured sync transports failed: {}", errors.join("; ")));
    }

    // D8: remote reviews may retire cards the open in-memory session still
    // offers as new — drop the batch so review_current rebuilds it.
    if reviews_received > 0 {
        invalidate_session(state);
    }

    Ok(json!({
        "reviewsSent": reviews_sent,
        "reviewsReceived": reviews_received,
        "settingsApplied": settings_applied,
        "viaIroh": via_iroh,
        "viaNostr": via_nostr,
        "errors": errors,
        "transports": transports,
    })
    .to_string())
}

/// nostr_identity_ensure() -> '"<pubkey hex>"'. Generates and persists an
/// identity when missing; when one already exists, derives the pubkey from
/// the stored secret. Idempotent: repeated calls return the SAME pubkey.
///
/// Reads through the secrets vault (schema v4) via the exported
/// `GreekCore::secret_get("nostr_nsec")` — the same location
/// `sync_generate_nostr_identity` / `sync_set_nostr_nsec` write. GreekCore
/// exposes identity generation but no sync-pubkey getter, so the derivation
/// runs the same Keys::parse the core itself uses (the stored value is the
/// secret key hex; parse also accepts nsec1… bech32). Do NOT resurrect the
/// old read of `user_settings.nostr_nsec` — that location is obsolete.
/// (If the core grows a sync-pubkey getter, switch to it and drop the
/// nostr-sdk derive here.)
pub fn nostr_identity_ensure(state: &AppState) -> Result<String, String> {
    if let Some(nsec) = state.core.secret_get("nostr_nsec".to_string()) {
        let keys = nostr_sdk::prelude::Keys::parse(nsec.trim())
            .map_err(|e| format!("stored nostr identity is invalid: {e}"))?;
        return Ok(Value::String(keys.public_key().to_hex()).to_string());
    }
    let pubkey = state.core.sync_generate_nostr_identity();
    Ok(Value::String(pubkey).to_string())
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
/// Wire versions v1+v2 enforced inside the core (see SYNC-WIRE.md).
///
/// D8: a successful import can mark this session's open cards reviewed on
/// another device — always drop the cached in-memory batch after a
/// successful import so review_current rebuilds it (even a 0-new-reviews
/// import: settings applied can change gating/budget inputs too).
pub fn backup_import(state: &AppState, json_text: &str) -> Result<String, String> {
    let stats = state
        .core
        .sync_import_backup_json(json_text.to_string())
        .map_err(|e| e.to_string())?;
    invalidate_session(state);
    Ok(json!({
        "reviewsReceived": stats.reviews_received,
        "settingsApplied": stats.settings_applied,
    })
    .to_string())
}
