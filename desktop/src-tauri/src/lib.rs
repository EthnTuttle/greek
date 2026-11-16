//! Greek desktop — Tauri shell over the shared Rust core (via the
//! uniffi-bindings crate as a plain Rust library). The invoke contract for
//! the UI lives in commands.rs; state lives in state.rs.

pub mod commands;
pub mod state;

use state::AppState;

#[tauri::command]
async fn home_stats(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::home_stats(&state.core)
}

#[tauri::command]
async fn review_current(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::review_current(&state).map(|v| v.to_string())
}

#[tauri::command]
async fn review_rate(state: tauri::State<'_, AppState>, rating: u8) -> Result<String, String> {
    commands::review_rate(&state, rating).map(|v| v.to_string())
}

#[tauri::command]
async fn curriculum(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::curriculum(&state.core)
}

#[tauri::command]
async fn alphabet_check_generate(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::alphabet_check_generate(&state.core)
}

#[tauri::command]
async fn alphabet_check_answer(
    state: tauri::State<'_, AppState>,
    card_id: String,
    correct: bool,
) -> Result<String, String> {
    commands::alphabet_check_answer(&state.core, &card_id, correct)
}

#[tauri::command]
async fn alphabet_check_verdict(
    state: tauri::State<'_, AppState>,
    correct: u32,
    total: u32,
) -> Result<String, String> {
    commands::alphabet_check_verdict(&state.core, correct, total)
}

#[tauri::command]
async fn alphabet_declare_known(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::alphabet_declare_known(&state.core)
}

#[tauri::command]
async fn chapter_unlock(
    state: tauri::State<'_, AppState>,
    textbook: String,
    chapter: i32,
) -> Result<String, String> {
    commands::chapter_unlock(&state.core, &textbook, chapter)
}

#[tauri::command]
async fn dict_search(state: tauri::State<'_, AppState>, q: String) -> Result<String, String> {
    commands::dict_search(&state.core, &q)
}

#[tauri::command]
async fn dict_status(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::dict_status(&state)
}

#[tauri::command]
async fn sync_status(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::sync_status(&state)
}

#[tauri::command]
async fn sync_ticket(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::sync_ticket(&state).await
}

#[tauri::command]
async fn sync_join(state: tauri::State<'_, AppState>, ticket: String) -> Result<String, String> {
    commands::sync_join(&state, &ticket).await
}

#[tauri::command]
async fn sync_now(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::sync_now(&state).await
}

#[tauri::command]
async fn nostr_identity_ensure(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::nostr_identity_ensure(&state)
}

#[tauri::command]
async fn nostr_relays_set(state: tauri::State<'_, AppState>, csv: String) -> Result<String, String> {
    commands::nostr_relays_set(&state.core, &csv)
}

#[tauri::command]
async fn backup_export(state: tauri::State<'_, AppState>) -> Result<String, String> {
    commands::backup_export(&state.core)
}

#[tauri::command]
async fn backup_import(state: tauri::State<'_, AppState>, json: String) -> Result<String, String> {
    commands::backup_import(&state, &json)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Resolve the Tauri bundle resource dir from the app handle BEFORE
        // .manage(state) (D2): it needs the handle, which only exists once
        // the Builder is running, so AppState is constructed here rather
        // than up front.
        .setup(|app| {
            use tauri::Manager;
            let resource_dir = app.path().resource_dir().ok();
            match AppState::from_env(resource_dir) {
                Ok(state) => {
                    app.manage(state);
                    Ok(())
                }
                Err(e) => Err(e.into()),
            }
        })
        .invoke_handler(tauri::generate_handler![
            home_stats,
            review_current,
            review_rate,
            curriculum,
            alphabet_check_generate,
            alphabet_check_answer,
            alphabet_check_verdict,
            alphabet_declare_known,
            chapter_unlock,
            dict_search,
            dict_status,
            sync_status,
            sync_ticket,
            sync_join,
            sync_now,
            nostr_identity_ensure,
            nostr_relays_set,
            backup_export,
            backup_import,
        ])
        .run(tauri::generate_context!())
        .expect("error while running greek-desktop");
}
