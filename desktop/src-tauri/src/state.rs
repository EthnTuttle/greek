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
    /// Resolved morphology.sqlite path, present only when the file exists
    /// and is non-empty (i.e. the dictionary is actually installed). Lets
    /// the UI distinguish "dictionary not installed" from "no matches" (D2).
    pub morph_path: Option<PathBuf>,
}

impl AppState {
    /// True when a morphology lexicon is installed (file present, non-empty)
    /// and was attached to the core at init.
    pub fn dictionary_installed(&self) -> bool {
        self.morph_path.is_some()
    }

    /// Build state from the environment:
    ///   GREEK_DATA_DIR    — overrides the data directory (XDG data dir/greek otherwise)
    ///   GREEK_MORPH_PATH  — overrides the morphology.sqlite location
    ///   GREEK_REPO_ASSETS — dev convenience: if it points at the repo's
    ///                       android/app/src/main/assets and morphology.sqlite
    ///                       is missing from the data dir, it is copied in.
    ///
    /// `resource_dir` is the Tauri bundle resource dir resolved from the app
    /// handle at Builder::setup (D2): the packaged morphology.sqlite ships
    /// there and outranks the data-dir default.
    pub fn from_env(resource_dir: Option<PathBuf>) -> Result<Self, String> {
        let data_dir = match env::var("GREEK_DATA_DIR") {
            Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => dirs::data_dir()
                .ok_or("cannot determine OS data directory")?
                .join("greek"),
        };
        Self::new_with_resources(data_dir, resource_dir)
    }

    /// Build state rooted at an explicit data directory (test-friendly).
    ///
    /// GreekCore::new applies schema v3 migrations, so firing it at a fresh
    /// database inside this directory is safe.
    pub fn new(data_dir: PathBuf) -> Result<Self, String> {
        Self::new_with_resources(data_dir, None)
    }

    /// Build state rooted at an explicit data directory with an explicit
    /// Tauri bundle resource dir (resolved by the caller from the app handle).
    pub fn new_with_resources(
        data_dir: PathBuf,
        resource_dir: Option<PathBuf>,
    ) -> Result<Self, String> {
        fs::create_dir_all(&data_dir)
            .map_err(|e| format!("cannot create {}: {e}", data_dir.display()))?;

        let env_override = env::var("GREEK_MORPH_PATH").ok();
        let repo_assets = env::var("GREEK_REPO_ASSETS").ok();
        let morph_path = resolve_morph_path(
            env_override.as_deref().filter(|p| !p.trim().is_empty()),
            resource_dir.as_deref(),
            &data_dir,
            repo_assets.as_deref(),
        )?;
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
            morph_path: Some(morph_path).filter(|p| file_present(p)),
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

/// Resolve the morphology.sqlite candidate path. Order (D2):
///   1. `env_override` (GREEK_MORPH_PATH) — explicit override always wins.
///   2. `<resource_dir>/morphology.sqlite` — the copy bundled by
///      `tauri.conf.json` resources; this is what fresh `.deb` installs get.
///   3. `<data_dir>/morphology.sqlite` — the default location; if missing
///      and `repo_assets` (GREEK_REPO_ASSETS) points at the repo's Android
///      assets dir, copy it in as a dev convenience.
/// The returned path may not exist; callers attach only when it does so the
/// dictionary degrades gracefully instead of fabricating an empty sqlite.
fn resolve_morph_path(
    env_override: Option<&str>,
    resource_dir: Option<&Path>,
    data_dir: &Path,
    repo_assets: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(p) = env_override {
        return Ok(PathBuf::from(p));
    }
    // Bundled copy first: a packaged install ships the pinned lexicon.
    if let Some(dir) = resource_dir {
        let bundled = dir.join("morphology.sqlite");
        if file_present(&bundled) {
            return Ok(bundled);
        }
    }
    let default = data_dir.join("morphology.sqlite");
    if !file_present(&default) {
        if let Some(assets) = repo_assets {
            let src = Path::new(assets).join("morphology.sqlite");
            if file_present(&src) {
                fs::copy(&src, &default).map_err(|e| {
                    format!(
                        "morphology copy {} -> {}: {e}",
                        src.display(),
                        default.display()
                    )
                })?;
            }
        }
    }
    Ok(default)
}

#[cfg(test)]
mod tests {
    //! Resource-resolution order tests use throwaway fake dirs (no GreekCore,
    //! so no valid sqlite is needed) and never touch process env vars so
    //! they're parallel-safe.
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    fn scratch() -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "greek-state-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let data = base.join("data");
        let res = base.join("res");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&res).unwrap();
        (data, res)
    }

    fn touch(p: &Path) {
        fs::write(p, b"x").unwrap(); // non-empty placeholder (never opened)
    }

    #[test]
    fn bundled_resource_wins_over_data_dir_default() {
        let (data, res) = scratch();
        touch(&res.join("morphology.sqlite"));
        touch(&data.join("morphology.sqlite"));
        let got = resolve_morph_path(None, Some(&res), &data, None).unwrap();
        assert_eq!(got, res.join("morphology.sqlite"));
        fs::remove_dir_all(data.parent().unwrap()).ok();
    }

    #[test]
    fn falls_back_to_data_dir_when_bundle_absent_or_empty() {
        let (data, res) = scratch();
        // Nothing staged in the (fake) resource dir → data-dir default.
        let got = resolve_morph_path(None, Some(&res), &data, None).unwrap();
        assert_eq!(got, data.join("morphology.sqlite"));
        // A zero-byte bundled copy is treated as absent.
        fs::write(res.join("morphology.sqlite"), b"").unwrap();
        touch(&data.join("morphology.sqlite"));
        let got = resolve_morph_path(None, Some(&res), &data, None).unwrap();
        assert_eq!(got, data.join("morphology.sqlite"));
        fs::remove_dir_all(data.parent().unwrap()).ok();
    }

    #[test]
    fn env_override_wins_over_bundle() {
        let (data, res) = scratch();
        touch(&res.join("morphology.sqlite"));
        let got =
            resolve_morph_path(Some("/elsewhere/morph.sqlite"), Some(&res), &data, None).unwrap();
        assert_eq!(got, PathBuf::from("/elsewhere/morph.sqlite"));
        fs::remove_dir_all(data.parent().unwrap()).ok();
    }
}
