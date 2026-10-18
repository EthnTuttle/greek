//! Build script: (1) merge all deck JSON files into a single cards.json in
//! OUT_DIR exactly like GreekApplication.kt does on Android (strip each
//! file's outer [] and comma-join, sorted filenames for determinism);
//! (2) run tauri_build::build().

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    merge_cards();
    tauri_build::build();
}

/// Concatenate <repo>/data/cards/*.json into $OUT_DIR/cards.json as one
/// JSON array. The app embeds the result with include_str!. The repo path
/// from this crate's manifest dir (desktop/src-tauri) is ../../data/cards.
fn merge_cards() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let cards_dir = manifest_dir.join("../../data/cards");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    let mut files: Vec<PathBuf> = fs::read_dir(&cards_dir)
        .unwrap_or_else(|e| panic!("cannot read cards dir {}: {e}", cards_dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    files.sort();

    let mut merged = String::with_capacity(4 * 1024 * 1024);
    merged.push('[');
    let mut first = true;
    for file in &files {
        let raw = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let trimmed = raw.trim();
        let inner = trimmed.strip_prefix('[').unwrap_or(trimmed);
        let inner = inner.strip_suffix(']').unwrap_or(inner).trim();
        if inner.is_empty() {
            continue;
        }
        if !first {
            merged.push(',');
        }
        merged.push_str(inner);
        first = false;
    }
    merged.push(']');

    fs::write(out_dir.join("cards.json"), &merged).expect("write cards.json");

    println!("cargo:rerun-if-changed={}", cards_dir.display());
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
    }
}
