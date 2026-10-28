use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use rusqlite::{params, Connection, OpenFlags};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Instant;
use unicode_normalization::UnicodeNormalization;

const BATCH_SIZE: usize = 10_000;
const PROGRESS_INTERVAL: usize = 100_000;

/// Default directory containing the Morpheus XML dumps. Override with
/// `--input-dir` or the `MORPHEUS_DATA_DIR` environment variable.
/// Deliberately NOT a location that exists on this machine: the original
/// inputs are lost, so every run must explicitly point at real inputs.
const DEFAULT_INPUT_DIR: &str = "data/inputs";

/// A database with more than this many rows in `lemmas` is considered a
/// healthy, fully-built morphology database. The pinned reference asset
/// (`data/morphology.sqlite`, SHA-256 pinned in `data/MANIFEST`) holds
/// ~36k lemmas and CANNOT be rebuilt (the source XMLs are lost), so the
/// pipeline refuses to overwrite such a file unless `--force` is given.
const MIN_HEALTHY_LEMMAS: i64 = 30_000;

const XML_FILE_NAMES: &[&str] = &[
    "MorpheusGreek1-319492.xml",
    "MorpheusGreek319493-638984.xml",
    "MorpheusGreek638985-958476.xml",
];

struct Args {
    input_dir: PathBuf,
    output: PathBuf,
    force: bool,
}

fn usage() -> String {
    format!(
        "Usage: data-pipeline [--input-dir DIR] [--output PATH] [--force]\n\
         \n\
         Builds the morphology SQLite database from the Morpheus XML dumps.\n\
         \n\
         Options:\n\
           --input-dir DIR   Directory containing the XML dumps\n\
                             (env MORPHEUS_DATA_DIR; default: {DEFAULT_INPUT_DIR})\n\
           --output PATH     Destination SQLite file (env MORPHOLOGY_OUTPUT;\n\
                             default: a path in the system temp dir — never the\n\
                             pinned data/morphology.sqlite asset)\n\
           --force           Allow overwriting an existing HEALTHY database.\n\
                             Required because the shipped morphology.sqlite is a\n\
                             pinned, un-rebuildable artifact (see data/MANIFEST).\n\
         \n\
         Safety: all expected input XMLs are validated (exist + non-empty)\n\
         before anything is written; the database is built into '<out>.tmp'\n\
         and atomically renamed over the destination only when fully built."
    )
}

/// Parse CLI arguments (zero-dependency: this crate intentionally uses
/// std::env rather than pulling in clap).
fn parse_args() -> Result<Args> {
    let mut input_dir: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut force = false;

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        if arg == "--input-dir" {
            let v = argv.next().context("--input-dir requires a value")?;
            input_dir = Some(PathBuf::from(v));
        } else if let Some(v) = arg.strip_prefix("--input-dir=") {
            input_dir = Some(PathBuf::from(v));
        } else if arg == "--output" || arg == "-o" {
            let v = argv.next().context("--output requires a value")?;
            output = Some(PathBuf::from(v));
        } else if let Some(v) = arg.strip_prefix("--output=") {
            output = Some(PathBuf::from(v));
        } else if arg == "--force" {
            force = true;
        } else if arg == "-h" || arg == "--help" {
            println!("{}", usage());
            std::process::exit(0);
        } else {
            anyhow::bail!("Unknown argument: {arg}\n{}", usage());
        }
    }

    let input_dir = input_dir
        .or_else(|| std::env::var_os("MORPHEUS_DATA_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_INPUT_DIR));
    let output = output
        .or_else(|| std::env::var_os("MORPHOLOGY_OUTPUT").map(PathBuf::from))
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("morphology-pipeline")
                .join("morphology.sqlite")
        });

    Ok(Args {
        input_dir,
        output,
        force,
    })
}

/// Validate that ALL expected input XMLs exist and are non-empty BEFORE any
/// output file is touched. Reports every missing/invalid input at once
/// rather than failing on the first one.
fn validate_inputs(input_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut problems: Vec<String> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();

    for name in XML_FILE_NAMES {
        let path = input_dir.join(name);
        match fs::metadata(&path) {
            Ok(md) if md.is_file() && md.len() > 0 => files.push(path),
            Ok(md) if md.is_file() => {
                problems.push(format!("{} (empty file)", path.display()))
            }
            Ok(_) => problems.push(format!("{} (not a regular file)", path.display())),
            Err(_) => problems.push(format!("{} (missing)", path.display())),
        }
    }

    if !problems.is_empty() {
        eprintln!("Input validation failed ({} problem(s)):", problems.len());
        for p in &problems {
            eprintln!("  {p}");
        }
        anyhow::bail!(
            "Refusing to build: {} of {} required Morpheus XML inputs are \
             missing or empty. No output was written.\n\
             (The original XML dumps are no longer obtainable; this check \
             exists because a build with missing inputs previously replaced \
             the pinned reference database with an empty one.)",
            problems.len(),
            XML_FILE_NAMES.len()
        );
    }

    Ok(files)
}

/// True if `path` is a healthy, fully-built morphology database: it opens,
/// contains the `lemmas` table, and holds more than MIN_HEALTHY_LEMMAS rows.
fn destination_is_healthy(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let conn = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => conn,
        Err(_) => return false,
    };
    let has_lemmas_table = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='lemmas'")
        .and_then(|mut stmt| stmt.exists([]))
        .unwrap_or(false);
    if !has_lemmas_table {
        return false;
    }
    let lemma_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM lemmas", [], |r| r.get(0))
        .unwrap_or(0);
    lemma_count > MIN_HEALTHY_LEMMAS
}

/// Scratch sibling path the database is built into before the atomic rename
/// over the real destination.
fn scratch_path(out: &Path) -> PathBuf {
    PathBuf::from(format!("{}.tmp", out.display()))
}

/// Maps first character of AGDT postag to part-of-speech label
fn postag_to_pos(postag: &str) -> &'static str {
    match postag.chars().next() {
        Some('n') => "noun",
        Some('v') => "verb",
        Some('a') => "adjective",
        Some('d') => "adverb",
        Some('p') => "pronoun",
        Some('l') => "article",
        Some('c') => "conjunction",
        Some('r') => "preposition",
        Some('e') => "exclamation",
        Some('m') => "numeral",
        Some('g') => "particle",
        Some('x') => "other",
        Some('-') => "unknown",
        _ => "unknown",
    }
}

/// Normalize Greek text: NFC normalization
fn normalize_greek(s: &str) -> String {
    s.nfc().collect()
}

#[derive(Debug, Clone)]
struct MorphEntry {
    form: String,
    form_normalized: String,
    lemma: String,
    lemma_normalized: String,
    postag: String,
    lemma_id: i64,
    definition: String,
}

/// Parse a single XML file, calling the callback for each entry
fn parse_xml_file(
    path: &Path,
    file_index: usize,
    mut on_entry: impl FnMut(MorphEntry),
) -> Result<(usize, usize)> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to open {}", path.display()))?;
    let reader = BufReader::with_capacity(1024 * 1024, file);
    let mut xml_reader = Reader::from_reader(reader);
    xml_reader.config_mut().trim_text(true);

    let mut buf = Vec::with_capacity(4096);
    let mut entries_parsed: usize = 0;
    let mut errors: usize = 0;

    // State machine for parsing
    #[derive(Default)]
    struct CompactEntry {
        form: String,
        form_bare: String,
        lemma: String,
        lemma_bare: String,
        postag: String,
        lemma_id: String,
        definition: String,
        current_tag: String,
    }

    #[derive(Default)]
    struct VerboseEntry {
        form: String,
        form_bare: String,
        lemma: String,
        lemma_bare: String,
        postag: String,
        lemma_id: String,
        definition: String,
        current_tag: String,
    }

    enum ParsingState {
        Idle,
        InCompactEntry(CompactEntry),
        InVerboseEntry(VerboseEntry),
    }

    let mut state = ParsingState::Idle;

    loop {
        match xml_reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match state {
                    ParsingState::Idle => {
                        if tag_name == "t" {
                            state = ParsingState::InCompactEntry(CompactEntry::default());
                        } else if tag_name == "token" {
                            state = ParsingState::InVerboseEntry(VerboseEntry::default());
                        }
                    }
                    ParsingState::InCompactEntry(ref mut entry) => {
                        entry.current_tag = tag_name;
                    }
                    ParsingState::InVerboseEntry(ref mut entry) => {
                        entry.current_tag = tag_name;
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                // Handle self-closing tags like <a/>
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match state {
                    ParsingState::InCompactEntry(ref mut entry) => {
                        let _ = tag_name;
                        entry.current_tag.clear();
                    }
                    ParsingState::InVerboseEntry(ref mut entry) => {
                        let _ = tag_name;
                        entry.current_tag.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                match state {
                    ParsingState::InCompactEntry(ref mut entry) => {
                        match entry.current_tag.as_str() {
                            "f" => entry.form = text,
                            "b" => entry.form_bare = text,
                            "l" => entry.lemma = text,
                            "e" => entry.lemma_bare = text,
                            "p" => entry.postag = text,
                            "d" => entry.lemma_id = text,
                            "s" => entry.definition = text,
                            _ => {}
                        }
                    }
                    ParsingState::InVerboseEntry(ref mut entry) => {
                        match entry.current_tag.as_str() {
                            "word-form" => entry.form = text,
                            "word-form-without-diacritics" => entry.form_bare = text,
                            "lemma" => entry.lemma = text,
                            "lemma-without-diacritics" => entry.lemma_bare = text,
                            "morphological-analysis" => entry.postag = text,
                            "lemma-id" => entry.lemma_id = text,
                            "translation" => entry.definition = text,
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match &state {
                    ParsingState::InCompactEntry(_) if tag_name == "t" => {
                        if let ParsingState::InCompactEntry(entry) =
                            std::mem::replace(&mut state, ParsingState::Idle)
                        {
                            if entry.form.is_empty()
                                || entry.lemma.is_empty()
                                || entry.postag.is_empty()
                                || entry.lemma_id.is_empty()
                            {
                                errors += 1;
                            } else {
                                match entry.lemma_id.parse::<i64>() {
                                    Ok(lid) => {
                                        entries_parsed += 1;
                                        on_entry(MorphEntry {
                                            form: entry.form,
                                            form_normalized: normalize_greek(&entry.form_bare),
                                            lemma: entry.lemma,
                                            lemma_normalized: normalize_greek(&entry.lemma_bare),
                                            postag: entry.postag,
                                            lemma_id: lid,
                                            definition: entry.definition,
                                        });
                                        if entries_parsed % PROGRESS_INTERVAL == 0 {
                                            eprintln!(
                                                "Processing file {}... {} entries parsed",
                                                file_index + 1,
                                                entries_parsed
                                            );
                                        }
                                    }
                                    Err(_) => {
                                        errors += 1;
                                    }
                                }
                            }
                        }
                    }
                    ParsingState::InVerboseEntry(_) if tag_name == "token" => {
                        if let ParsingState::InVerboseEntry(entry) =
                            std::mem::replace(&mut state, ParsingState::Idle)
                        {
                            if entry.form.is_empty()
                                || entry.lemma.is_empty()
                                || entry.postag.is_empty()
                                || entry.lemma_id.is_empty()
                            {
                                errors += 1;
                            } else {
                                match entry.lemma_id.parse::<i64>() {
                                    Ok(lid) => {
                                        entries_parsed += 1;
                                        on_entry(MorphEntry {
                                            form: entry.form,
                                            form_normalized: normalize_greek(&entry.form_bare),
                                            lemma: entry.lemma,
                                            lemma_normalized: normalize_greek(&entry.lemma_bare),
                                            postag: entry.postag,
                                            lemma_id: lid,
                                            definition: entry.definition,
                                        });
                                    }
                                    Err(_) => {
                                        errors += 1;
                                    }
                                }
                            }
                        }
                    }
                    ParsingState::InCompactEntry(_) => {
                        if let ParsingState::InCompactEntry(ref mut entry) = state {
                            entry.current_tag.clear();
                        }
                    }
                    ParsingState::InVerboseEntry(_) => {
                        if let ParsingState::InVerboseEntry(ref mut entry) = state {
                            entry.current_tag.clear();
                        }
                    }
                    _ => {}
                }
            }
            Err(e) => {
                eprintln!(
                    "XML parse error at position {}: {:?}",
                    xml_reader.error_position(),
                    e
                );
                errors += 1;
            }
            _ => {}
        }
        buf.clear();
    }

    Ok((entries_parsed, errors))
}

fn create_database(scratch_db_path: &Path) -> Result<Connection> {
    // `scratch_db_path` is our own "<out>.tmp" build scratch file — never the
    // real destination. A stale one may remain from a crashed run; removing
    // it is safe.
    if scratch_db_path.exists() {
        fs::remove_file(scratch_db_path).with_context(|| {
            format!(
                "Failed to remove stale scratch database {}",
                scratch_db_path.display()
            )
        })?;
    }

    let conn = Connection::open(scratch_db_path)?;

    // Performance pragmas for bulk import
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = OFF;
         PRAGMA cache_size = -64000;
         PRAGMA temp_store = MEMORY;
         PRAGMA mmap_size = 268435456;",
    )?;

    // Create schema
    conn.execute_batch(
        "CREATE TABLE lemmas(
           id INTEGER PRIMARY KEY,
           headword TEXT NOT NULL,
           headword_normalized TEXT NOT NULL,
           pos TEXT,
           definition TEXT,
           frequency_rank INTEGER DEFAULT 0
         );

         CREATE TABLE forms(
           form TEXT NOT NULL,
           form_normalized TEXT NOT NULL,
           lemma_id INTEGER NOT NULL REFERENCES lemmas(id),
           postag TEXT NOT NULL
         );",
    )?;

    Ok(conn)
}

/// Parse all XML inputs and populate an already-created database.
/// Returns (lemma_count, form_count, parse_errors).
fn build_database(conn: &Connection, xml_files: &[PathBuf]) -> Result<(i64, i64, usize)> {

    // Collect all entries, deduplicating as we go
    // Key: (form, lemma_id, postag) -> only insert once
    let mut forms_seen: HashSet<(String, i64, String)> = HashSet::with_capacity(1_000_000);
    // Lemma data: lemma_id -> (headword, headword_normalized, pos, definition)
    let mut lemmas: HashMap<i64, (String, String, String, String)> = HashMap::with_capacity(50_000);
    // Forms to insert: Vec of (form, form_normalized, lemma_id, postag)
    let mut forms_to_insert: Vec<(String, String, i64, String)> = Vec::with_capacity(1_000_000);

    let mut total_entries = 0usize;
    let mut total_errors = 0usize;

    for (i, path) in xml_files.iter().enumerate() {
        eprintln!(
            "Processing file {}... ({})",
            i + 1,
            path.file_name().unwrap().to_string_lossy()
        );

        let (entries, errors) = parse_xml_file(path, i, |entry| {
            let key = (entry.form.clone(), entry.lemma_id, entry.postag.clone());

            if forms_seen.contains(&key) {
                return;
            }
            forms_seen.insert(key);

            // Update lemma info (first occurrence wins for definition)
            lemmas.entry(entry.lemma_id).or_insert_with(|| {
                let pos = postag_to_pos(&entry.postag).to_string();
                (
                    entry.lemma.clone(),
                    entry.lemma_normalized.clone(),
                    pos,
                    entry.definition.clone(),
                )
            });

            forms_to_insert.push((
                entry.form,
                entry.form_normalized,
                entry.lemma_id,
                entry.postag,
            ));
        })?;

        eprintln!(
            "  File {} done: {} entries parsed, {} errors",
            i + 1,
            entries,
            errors
        );
        total_entries += entries;
        total_errors += errors;
    }

    let total_duplicates = total_entries - forms_to_insert.len();
    eprintln!(
        "\nTotal: {} entries parsed, {} unique forms, {} duplicates removed, {} errors",
        total_entries,
        forms_to_insert.len(),
        total_duplicates,
        total_errors
    );

    // Insert lemmas
    eprintln!("Inserting {} lemmas...", lemmas.len());
    {
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO lemmas (id, headword, headword_normalized, pos, definition) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;

            for (id, (headword, headword_normalized, pos, definition)) in &lemmas {
                stmt.execute(params![id, headword, headword_normalized, pos, definition])?;
            }
        }
        tx.commit()?;
    }
    eprintln!("  Lemmas inserted.");

    // Insert forms in batches
    eprintln!("Inserting {} forms...", forms_to_insert.len());
    {
        let mut inserted = 0usize;
        for chunk in forms_to_insert.chunks(BATCH_SIZE) {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO forms (form, form_normalized, lemma_id, postag) VALUES (?1, ?2, ?3, ?4)",
                )?;
                for (form, form_normalized, lemma_id, postag) in chunk {
                    stmt.execute(params![form, form_normalized, lemma_id, postag])?;
                }
            }
            tx.commit()?;
            inserted += chunk.len();
            if inserted % (BATCH_SIZE * 10) == 0 {
                eprintln!("  {} forms inserted...", inserted);
            }
        }
    }
    eprintln!("  Forms inserted.");

    // Compute frequency_rank: count of distinct forms per lemma
    eprintln!("Computing frequency ranks...");
    conn.execute_batch(
        "UPDATE lemmas SET frequency_rank = (
           SELECT COUNT(*) FROM forms WHERE forms.lemma_id = lemmas.id
         );",
    )?;
    eprintln!("  Frequency ranks computed.");

    // Create indexes
    eprintln!("Creating indexes...");
    conn.execute_batch(
        "CREATE INDEX idx_forms_form ON forms(form);
         CREATE INDEX idx_forms_normalized ON forms(form_normalized);
         CREATE INDEX idx_lemmas_headword ON lemmas(headword);",
    )?;
    eprintln!("  Indexes created.");

    // Create and populate FTS5 table
    eprintln!("Creating FTS5 index...");
    conn.execute_batch(
        "CREATE VIRTUAL TABLE lemmas_fts USING fts5(headword, definition, content=lemmas, content_rowid=id);",
    )?;
    conn.execute_batch(
        "INSERT INTO lemmas_fts(rowid, headword, definition) SELECT id, headword, definition FROM lemmas;",
    )?;
    eprintln!("  FTS5 index created.");

    // Summary counts
    let lemma_count: i64 = conn.query_row("SELECT COUNT(*) FROM lemmas", [], |r| r.get(0))?;
    let form_count: i64 = conn.query_row("SELECT COUNT(*) FROM forms", [], |r| r.get(0))?;

    Ok((lemma_count, form_count, total_errors))
}

fn main() -> Result<()> {
    let start = Instant::now();
    let args = parse_args()?;

    eprintln!("Input dir: {}", args.input_dir.display());
    eprintln!("Output:    {}", args.output.display());

    // 1. Validate ALL inputs exist and are non-empty BEFORE anything is
    //    created or deleted on the output side.
    let xml_files = validate_inputs(&args.input_dir)?;

    // 2. Guard: never implicitly overwrite a healthy, fully-built database.
    //    The shipped data/morphology.sqlite is pinned (data/MANIFEST) and
    //    cannot be rebuilt — the source XMLs are lost — so replacing it must
    //    always be an explicit, deliberate act.
    if !args.force && destination_is_healthy(&args.output) {
        anyhow::bail!(
            "Refusing to overwrite {}: it is a healthy morphology database \
             (more than {} lemmas).\n\
             If this is the pinned data/morphology.sqlite, overwriting it \
             would destroy an un-rebuildable artifact (see data/MANIFEST). \
             Pass --force only if you are deliberately replacing it.",
            args.output.display(),
            MIN_HEALTHY_LEMMAS
        );
    }

    // 3. Build into a scratch sibling "<out>.tmp"; the destination is only
    //    replaced by a fully-built database via an atomic rename.
    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let tmp_path = scratch_path(&args.output);
    eprintln!("Building scratch database at {}", tmp_path.display());

    let conn = create_database(&tmp_path)?;
    let stats = match build_database(&conn, &xml_files) {
        Ok(stats) => stats,
        Err(e) => {
            // Leave the destination untouched and don't leave scratch residue.
            drop(conn);
            let _ = fs::remove_file(&tmp_path);
            let _ = fs::remove_file(format!("{}.tmp-wal", args.output.display()));
            let _ = fs::remove_file(format!("{}.tmp-shm", args.output.display()));
            return Err(e);
        }
    };
    let (lemma_count, form_count, total_errors) = stats;

    // Fold the WAL back into the main file so the scratch file is
    // self-contained, then fsync before the swap.
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    conn.close().map_err(|(_, e)| e)?;
    fs::File::open(&tmp_path)?.sync_all()?;

    fs::rename(&tmp_path, &args.output).with_context(|| {
        format!(
            "Failed to move {} into place as {}",
            tmp_path.display(),
            args.output.display()
        )
    })?;
    eprintln!("Installed {} -> {}", tmp_path.display(), args.output.display());

    let elapsed = start.elapsed();
    eprintln!("\nDone in {:.1}s", elapsed.as_secs_f64());
    eprintln!("Database: {}", args.output.display());
    eprintln!("  {} lemmas, {} forms", lemma_count, form_count);

    if total_errors > 0 {
        eprintln!("  {} entries skipped due to parse errors", total_errors);
    }

    Ok(())
}
