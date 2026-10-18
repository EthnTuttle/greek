use rusqlite::Connection;
use serde::{Deserialize, Serialize};

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
            }) {
                for row in rows.flatten() {
                    results.push(row);
                }
            }
        }

        results
    }
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
        assert_eq!(
            sanitize_fts5_query("house OR garden"),
            "house OR garden"
        );
    }

    #[test]
    fn test_search_engine_creation() {
        let _engine = SearchEngine::new();
    }
}
