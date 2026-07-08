use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::time::Instant;
use unicode_normalization::UnicodeNormalization;

const BATCH_SIZE: usize = 10_000;
const PROGRESS_INTERVAL: usize = 100_000;

const XML_FILES: &[&str] = &[
    "/tmp/morpheus-data/MorpheusGreek1-319492.xml",
    "/tmp/morpheus-data/MorpheusGreek319493-638984.xml",
    "/tmp/morpheus-data/MorpheusGreek638985-958476.xml",
];

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

fn create_database(db_path: &Path) -> Result<Connection> {
    if db_path.exists() {
        fs::remove_file(db_path)?;
    }

    let conn = Connection::open(db_path)?;

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

fn main() -> Result<()> {
    let start = Instant::now();

    // Create data directory
    let data_dir = Path::new("/home/ethan/code/greek/data");
    fs::create_dir_all(data_dir)?;

    let db_path = data_dir.join("morphology.sqlite");
    let conn = create_database(&db_path)?;

    eprintln!("Database created at {}", db_path.display());

    // Collect all entries, deduplicating as we go
    // Key: (form, lemma_id, postag) -> only insert once
    let mut forms_seen: HashSet<(String, i64, String)> = HashSet::with_capacity(1_000_000);
    // Lemma data: lemma_id -> (headword, headword_normalized, pos, definition)
    let mut lemmas: HashMap<i64, (String, String, String, String)> = HashMap::with_capacity(50_000);
    // Forms to insert: Vec of (form, form_normalized, lemma_id, postag)
    let mut forms_to_insert: Vec<(String, String, i64, String)> = Vec::with_capacity(1_000_000);

    let mut total_entries = 0usize;
    let mut total_errors = 0usize;

    for (i, file_path) in XML_FILES.iter().enumerate() {
        let path = Path::new(file_path);
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

    // Final stats
    let elapsed = start.elapsed();
    eprintln!("\nDone in {:.1}s", elapsed.as_secs_f64());
    eprintln!("Database: {}", db_path.display());

    // Print summary counts
    let lemma_count: i64 = conn.query_row("SELECT COUNT(*) FROM lemmas", [], |r| r.get(0))?;
    let form_count: i64 = conn.query_row("SELECT COUNT(*) FROM forms", [], |r| r.get(0))?;
    eprintln!("  {} lemmas, {} forms", lemma_count, form_count);

    if total_errors > 0 {
        eprintln!("  {} entries skipped due to parse errors", total_errors);
    }

    Ok(())
}
