use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::morphology::normalize_form;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub lemma_id: i64,
    pub headword: String,
    pub definition: String,
    pub pos: String,
    pub rank: f64,
}

pub struct SearchEngine;

impl SearchEngine {
    pub fn new() -> Self {
        Self
    }

    /// Search the dictionary by headword or definition text.
    /// Uses FTS5 on the morphology database.
    pub fn search(&self, conn: &Connection, query: &str, limit: usize) -> Vec<SearchResult> {
        // Sanitize query for FTS5 (escape special characters)
        let sanitized = sanitize_fts5_query(query);
        if sanitized.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();

        // Relevance boost: bm25 buries core vocabulary when a common English
        // term appears inside many compound-word glosses (query "man" ranked
        // ἄνθρωπος "man, human being" below 25 "man-slaying"-style entries).
        // Exact and first-gloss matches sort before everything bm25 finds.
        let needle = query.trim().to_lowercase();
        let first_gloss = format!("{},*", needle);

        if let Ok(mut stmt) = conn.prepare(
            "SELECT l.id, l.headword, l.definition, l.pos, rank
             FROM morph.lemmas_fts
             JOIN morph.lemmas l ON l.id = lemmas_fts.rowid
             WHERE lemmas_fts MATCH ?1
             ORDER BY CASE
                 WHEN lower(l.definition) = ?3 THEN 0
                 WHEN lower(l.definition) GLOB ?4 THEN 1
                 ELSE 2
             END, rank
             LIMIT ?2",
        ) {
            if let Ok(rows) = stmt.query_map(
                rusqlite::params![sanitized, limit as i64, needle, first_gloss],
                |row| {
                    Ok(SearchResult {
                        lemma_id: row.get(0)?,
                        headword: row.get(1)?,
                        definition: row.get(2)?,
                        pos: row.get(3)?,
                        rank: row.get(4)?,
                    })
                },
            ) {
                for row in rows.flatten() {
                    results.push(row);
                }
            }
        }

        // FTS stores the accented surface headword, so an unaccented Greek
        // query commonly has no MATCH rows. Fall back to the DB's bare
        // headword column for equality/prefix matching. The shared normalizer
        // strips tonos/oxia/dialytika and folds case; final sigma is folded on
        // both sides because the stored column preserves σ/ς distinctions.
        if results.is_empty() {
            let normalized = normalize_headword_query(query);
            let prefix = format!("{}%", escape_like_pattern(&normalized));
            // REPLACE/LIKE cannot use the surface-headword index, but this
            // bounded scan is over only ~36k lemmas and runs solely when FTS
            // misses. Keeping the shipped DB/index unchanged is preferable to
            // adding a second generated normalization/index to maintain.
            if let Ok(mut stmt) = conn.prepare(
                "SELECT l.id, l.headword, l.definition, l.pos, 0.0 AS rank
                 FROM morph.lemmas l
                 WHERE replace(l.headword_normalized, 'ς', 'σ') = ?1
                    OR replace(l.headword_normalized, 'ς', 'σ') LIKE ?2 ESCAPE '\\'
                 ORDER BY CASE
                     WHEN replace(l.headword_normalized, 'ς', 'σ') = ?1 THEN 0
                     ELSE 1
                 END, l.headword
                 LIMIT ?3",
            ) {
                if let Ok(rows) =
                    stmt.query_map(rusqlite::params![normalized, prefix, limit as i64], |row| {
                        Ok(SearchResult {
                            lemma_id: row.get(0)?,
                            headword: row.get(1)?,
                            definition: row.get(2)?,
                            pos: row.get(3)?,
                            rank: row.get(4)?,
                        })
                    })
                {
                    results.extend(rows.flatten());
                }
            }
        }

        results
    }
}

fn normalize_headword_query(query: &str) -> String {
    normalize_form(query.trim()).replace('ς', "σ")
}

/// Treat LIKE metacharacters literally; the only wildcard is the trailing '%'
/// appended by the lookup to implement prefix search.
fn escape_like_pattern(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Sanitize a query string for FTS5 MATCH syntax.
/// Wraps each token in quotes to prevent FTS5 syntax errors.
fn sanitize_fts5_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // If it looks like the user is using FTS5 syntax already, pass through
    if trimmed.contains('"') || trimmed.contains('*') || trimmed.contains("OR") {
        return trimmed.to_string();
    }

    // Otherwise, quote each word to avoid syntax issues with special chars
    trimmed
        .split_whitespace()
        .map(|word| format!("\"{}\"", word.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_fts5_query_simple() {
        assert_eq!(sanitize_fts5_query("house"), "\"house\"");
    }

    #[test]
    fn test_sanitize_fts5_query_multiple_words() {
        assert_eq!(sanitize_fts5_query("big house"), "\"big\" \"house\"");
    }

    #[test]
    fn test_sanitize_fts5_query_empty() {
        assert_eq!(sanitize_fts5_query(""), "");
        assert_eq!(sanitize_fts5_query("   "), "");
    }

    #[test]
    fn test_sanitize_fts5_query_passthrough() {
        // Already using FTS5 syntax
        assert_eq!(sanitize_fts5_query("house*"), "house*");
        assert_eq!(sanitize_fts5_query("house OR garden"), "house OR garden");
    }

    fn dictionary_fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "ATTACH DATABASE ':memory:' AS morph;
             CREATE TABLE morph.lemmas(
                 id INTEGER PRIMARY KEY,
                 headword TEXT NOT NULL,
                 headword_normalized TEXT NOT NULL,
                 pos TEXT,
                 definition TEXT
             );
             CREATE VIRTUAL TABLE morph.lemmas_fts USING fts5(
                 headword, definition, content=lemmas, content_rowid=id
             );
             INSERT INTO morph.lemmas VALUES
                 (1, 'λόγος', 'λογος', 'noun', 'word, speech'),
                 (2, 'ἄνθρωπος', 'ανθρωπος', 'noun', 'man, human being');
             INSERT INTO morph.lemmas_fts(rowid, headword, definition)
                 SELECT id, headword, definition FROM morph.lemmas;",
        )
        .unwrap();
        conn
    }

    fn contains_headword(results: &[SearchResult], expected: &str) -> bool {
        results.iter().any(|r| r.headword == expected)
    }

    #[test]
    fn test_unaccented_headword_search() {
        let conn = dictionary_fixture();
        let engine = SearchEngine::new();
        let results = engine.search(&conn, "λογος", 10);
        assert!(contains_headword(&results, "λόγος"), "{results:?}");
        let results = engine.search(&conn, "ανθρωπος", 10);
        assert!(contains_headword(&results, "ἄνθρωπος"), "{results:?}");
    }

    #[test]
    fn test_final_sigma_and_prefix_search() {
        let conn = dictionary_fixture();
        let engine = SearchEngine::new();
        for query in ["λογος", "λογοσ", "ΛΟΓΟΣ", "λογ"] {
            let results = engine.search(&conn, query, 10);
            assert!(
                contains_headword(&results, "λόγος"),
                "query {query:?} failed: {results:?}"
            );
        }
    }

    #[test]
    fn test_accented_headword_search_still_works() {
        let conn = dictionary_fixture();
        let engine = SearchEngine::new();
        for (query, expected) in [("λόγος", "λόγος"), ("ἄνθρωπος", "ἄνθρωπος")]
        {
            let results = engine.search(&conn, query, 10);
            assert!(
                contains_headword(&results, expected),
                "query {query:?} failed: {results:?}"
            );
        }
    }

    #[test]
    fn test_search_engine_creation() {
        let _engine = SearchEngine::new();
    }
}
