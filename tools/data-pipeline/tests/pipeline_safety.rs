//! Safety regression tests for the hardening of the morphology data
//! pipeline (audit item R2). Previously the binary deleted the destination
//! database up front and created a new empty one before even attempting to
//! open the (lost) source XMLs, so a run with missing inputs replaced the
//! pinned, un-rebuildable `data/morphology.sqlite` with an empty database.
//!
//! These tests run the actual binary against scratch directories and assert:
//! * missing/empty inputs cause a nonzero exit that reports every bad input,
//! * the destination file is left byte-for-byte untouched (same bytes, same
//!   mtime) and no `<dest>.tmp` scratch residue remains,
//! * a healthy destination (> 30k lemmas) is never overwritten without
//!   `--force`, and `--force` performs a real atomic replacement.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const XML_FILE_NAMES: [&str; 3] = [
    "MorpheusGreek1-319492.xml",
    "MorpheusGreek319493-638984.xml",
    "MorpheusGreek638985-958476.xml",
];

const SENTINEL: &[u8] = b"pinned-database-sentinel-bytes";

/// Unique scratch directory under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let unique = format!(
            "data-pipeline-tests-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn join(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(inputs: &Path, dest: &Path, force: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_data-pipeline"));
    cmd.arg("--input-dir").arg(inputs);
    cmd.arg("--output").arg(dest);
    if force {
        cmd.arg("--force");
    }
    cmd.output().expect("failed to spawn data-pipeline binary")
}

fn scratch_db_path(dest: &Path) -> PathBuf {
    PathBuf::from(format!("{}.tmp", dest.display()))
}

/// Minimal well-formed Morpheus XML fragment with one parseable compact entry.
fn xml_doc(lemma_id: i64) -> String {
    format!(
        "<root><t><f>form{lemma_id}</f><b>form{lemma_id}</b><l>lemma{lemma_id}</l>\
         <e>lemma{lemma_id}</e><p>n---</p><d>{lemma_id}</d><s>gloss {lemma_id}</s></t></root>"
    )
}

fn write_valid_inputs(inputs: &Path) {
    for (i, name) in XML_FILE_NAMES.iter().enumerate() {
        fs::write(inputs.join(name), xml_doc(100 + i as i64)).expect("write input xml");
    }
}

/// Create a destination database that passes the health guard
/// (lemmas table with more than 30k rows).
fn make_healthy_db(dest: &Path) {
    let conn = rusqlite::Connection::open(dest).expect("create healthy db");
    conn.execute_batch(
        "CREATE TABLE lemmas(
           id INTEGER PRIMARY KEY,
           headword TEXT NOT NULL,
           headword_normalized TEXT NOT NULL,
           pos TEXT,
           definition TEXT,
           frequency_rank INTEGER DEFAULT 0
         );",
    )
    .expect("create schema");
    {
        let tx = conn.unchecked_transaction().expect("begin tx");
        {
            let mut stmt = tx
                .prepare("INSERT INTO lemmas (id, headword, headword_normalized) VALUES (?1, ?2, ?2)")
                .expect("prepare insert");
            for id in 1..=30_001i64 {
                stmt.execute(rusqlite::params![id, format!("w{id}")])
                    .expect("insert lemma");
            }
        }
        tx.commit().expect("commit tx");
    }
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
    conn.close().map_err(|(_, e)| e).expect("close db");
}

#[test]
fn missing_inputs_fail_and_leave_destination_untouched() {
    let scratch = Scratch::new("missing-inputs");
    let inputs = scratch.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    let dest = scratch.join("morphology.sqlite");
    fs::write(&dest, SENTINEL).unwrap();
    let before_bytes = fs::read(&dest).unwrap();
    let before_mtime = fs::metadata(&dest).unwrap().modified().unwrap();

    let output = run(&inputs, &dest, false);

    assert!(
        !output.status.success(),
        "expected nonzero exit, got {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for name in XML_FILE_NAMES {
        assert!(
            stderr.contains(name),
            "stderr should report missing input {name}:\n{stderr}"
        );
    }
    assert_eq!(
        fs::read(&dest).unwrap(),
        before_bytes,
        "destination bytes changed"
    );
    assert_eq!(
        fs::metadata(&dest).unwrap().modified().unwrap(),
        before_mtime,
        "destination mtime changed"
    );
    assert!(
        !scratch_db_path(&dest).exists(),
        "scratch '<dest>.tmp' residue left behind"
    );
}

#[test]
fn empty_and_partially_missing_inputs_are_all_reported() {
    let scratch = Scratch::new("empty-inputs");
    let inputs = scratch.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    // Two files exist but are empty; the third is missing entirely.
    fs::write(inputs.join(XML_FILE_NAMES[0]), "").unwrap();
    fs::write(inputs.join(XML_FILE_NAMES[1]), "").unwrap();
    let dest = scratch.join("morphology.sqlite");
    fs::write(&dest, SENTINEL).unwrap();

    let output = run(&inputs, &dest, false);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    for name in XML_FILE_NAMES {
        assert!(
            stderr.contains(name),
            "stderr should report problem input {name}:\n{stderr}"
        );
    }
    assert!(stderr.contains("empty"), "stderr should call out empty files:\n{stderr}");
    assert_eq!(fs::read(&dest).unwrap(), SENTINEL);
    assert!(!scratch_db_path(&dest).exists());
}

#[test]
fn healthy_destination_requires_force() {
    let scratch = Scratch::new("guard");
    let inputs = scratch.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    write_valid_inputs(&inputs);
    let dest = scratch.join("morphology.sqlite");
    make_healthy_db(&dest);
    let before_bytes = fs::read(&dest).unwrap();

    // Without --force: refused, destination untouched.
    let output = run(&inputs, &dest, false);
    assert!(!output.status.success(), "guard should refuse overwrite");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--force"),
        "refusal should mention --force:\n{stderr}"
    );
    assert_eq!(
        fs::read(&dest).unwrap(),
        before_bytes,
        "healthy destination modified without --force"
    );
    assert!(!scratch_db_path(&dest).exists());
}

#[test]
fn force_replaces_healthy_destination_via_atomic_rename() {
    let scratch = Scratch::new("force");
    let inputs = scratch.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    write_valid_inputs(&inputs);
    let dest = scratch.join("morphology.sqlite");
    make_healthy_db(&dest);

    let output = run(&inputs, &dest, true);
    assert!(
        output.status.success(),
        "--force rebuild failed, status {:?}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !scratch_db_path(&dest).exists(),
        "scratch file should have been renamed away, not left behind"
    );

    // Destination is now the freshly built 3-lemma database (one entry per
    // input file), i.e. it was replaced atomically by the finished build.
    let conn = rusqlite::Connection::open(&dest).expect("open rebuilt db");
    let lemma_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM lemmas", [], |r| r.get(0))
        .expect("count lemmas");
    let form_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM forms", [], |r| r.get(0))
        .expect("count forms");
    assert_eq!(lemma_count, 3);
    assert_eq!(form_count, 3);
}
