//! App state: the shared GreekCore (from the uniffi-bindings crate, used as
//! a plain Rust library) plus the in-memory review session.

use std::collections::VecDeque;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use uniffi_bindings::GreekCore;

/// All card decks merged by build.rs into one JSON array (same order and
/// shape as the Android asset merge in GreekApplication.kt).
pub const CARDS_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cards.json"));

/// One queued review item (due card or newly introduced card).
#[derive(Debug, Clone)]
pub struct SessionCard {
    pub card_id: String,
    pub front: String,
    pub back: String,
    pub is_new: bool,
}

/// In-memory review batch. Unlike Android (which persists the batch to
/// DataStore so it survives process death), the desktop session is
/// deliberately in-memory only per the wave-7 spec.
#[derive(Debug, Default)]
pub struct ReviewSession {
    pub queue: VecDeque<SessionCard>,
}

/// Tauri-managed application state.
pub struct AppState {
    pub core: Arc<GreekCore>,
    pub session: Mutex<ReviewSession>,
    /// Directory holding greek_app.db, morphology.sqlite, iroh-sync/.
    pub data_dir: PathBuf,
    /// Full path of the app database (greek_app.db inside data_dir).
    pub db_path: PathBuf,
}

impl AppState {
    /// Build state from the environment:
    ///   GREEK_DATA_DIR    — overrides the data directory (XDG data dir/greek otherwise)
    ///   GREEK_MORPH_PATH  — overrides the morphology.sqlite location
    ///   GREEK_REPO_ASSETS — dev convenience: if it points at the repo's
    ///                       android/app/src/main/assets and morphology.sqlite
    ///                       is missing from the data dir, it is copied in.
    pub fn from_env() -> Result<Self, String> {
        let data_dir = match env::var("GREEK_DATA_DIR") {
            Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => dirs::data_dir()
                .ok_or("cannot determine OS data directory")?
                .join("greek"),
        };
        Self::new(data_dir)
    }

    /// Build state rooted at an explicit data directory (test-friendly).
    ///
    /// GreekCore::new applies schema v3 migrations, so firing it at a fresh
    /// database inside this directory is safe.
    pub fn new(data_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&data_dir)
            .map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;

        // Morphology lexicon: env override first, else <data_dir>/morphology.sqlite.
        // If absent and GREEK_REPO_ASSETS points at the repo's Android assets
        // dir, copy it in as a dev convenience. Without it the dictionary
        // degrades gracefully: dict_search simply returns []. (Production
        // packaging ships the sqlite via Tauri resources — rollout item.)
        let morph_path = match env::var("GREEK_MORPH_PATH") {
            Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => {
                let default = data_dir.join("morphology.sqlite");
                if !file_present(&default) {
                    if let Ok(assets) = env::var("GREEK_REPO_ASSETS") {
                        let src = Path::new(&assets).join("morphology.sqlite");
                        if file_present(&src) {
                            fs::copy(&src, &default).map_err(|e| {
                                format!("morphology copy {} -> {}: {e}", src.display(), default.display())
                            })?;
                        }
                    }
                }
                default
            }
        };
        // Only attach when the file actually exists and is non-empty —
        // SQLite ATTACH would otherwise create an empty file and mask the
        // "not installed" state.
        let morph_arg = if file_present(&morph_path) {
            Some(morph_path.to_string_lossy().into_owned())
        } else {
            None
        };

        let db_path = data_dir.join("greek_app.db");
        let core = GreekCore::new_with_cards(
            db_path.to_string_lossy().into_owned(),
            morph_arg,
            CARDS_JSON.to_string(),
        )
        .map_err(|e| format!("GreekCore init failed: {e}"))?;

        Ok(Self {
            core,
            session: Mutex::new(ReviewSession::default()),
            data_dir,
            db_path,
        })
    }

    /// True once an iroh sync notebook has been opened on this machine
    /// (ticket created or joined). The core lazily creates <db dir>/iroh-sync
    /// on first sync_iroh_* call, so directory presence is the persisted
    /// "iroh configured" signal — mirroring Android's UI-side
    /// SYNC_IROH_JOINED datastore flag, but derived from core state.
    pub fn iroh_configured(&self) -> bool {
        self.data_dir.join("iroh-sync").is_dir()
    }
}

fn file_present(p: &Path) -> bool {
    p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
}
