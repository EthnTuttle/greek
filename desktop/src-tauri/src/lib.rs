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
async fn dict_search(state: tauri::State<'_, AppState>, q: String) -> Result<String, String> {
    commands::dict_search(&state.core, &q)
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
    commands::backup_import(&state.core, &json)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = match AppState::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("greek-desktop: fatal init error: {e}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            home_stats,
            review_current,
            review_rate,
            curriculum,
            dict_search,
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
