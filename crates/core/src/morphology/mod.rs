use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Normalize a Greek surface form to match the DB's accent-stripped,
/// lowercased `form_normalized` column: NFD-decompose, drop combining
/// diacritical marks (Unicode category Mn), then lowercase and recompose.
/// Public so other layers (e.g. typed-answer grading) reuse the SAME
/// normalization instead of duplicating it.
pub fn normalize_form(form: &str) -> String {
    form.nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
        .nfc()
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    pub lemma: String,
    pub lemma_id: i64,
    pub postag: String,
    pub postag_human: String,
    pub definition: String,
    pub confidence: f32,
    /// Best available part of speech: stored lemma POS unless it is
    /// uninformative, otherwise the POS encoded in the representative postag.
    pub pos: String,
}

/// Running Greek replaces a final acute with a grave. Morpheus stores the
/// citation-like acute form, so undo that contextual accent change before the
/// exact lookup. NFD also covers both precomposed and decomposed input.
fn grave_to_acute(form: &str) -> String {
    form.nfd()
        .map(|c| if c == '\u{0300}' { '\u{0301}' } else { c })
        .nfc()
        .collect()
}

fn postag_pos(postag: &str) -> &'static str {
    match postag.chars().next() {
        Some('n') => "noun",
        Some('v') => "verb",
        Some('t') => "participle",
        Some('a') => "adjective",
        Some('d') => "adverb",
        Some('l') => "article",
        Some('p') => "pronoun",
        Some('c') => "conjunction",
        Some('r') => "preposition",
        Some('e') => "exclamation",
        Some('m') => "numeral",
        Some('g') => "particle",
        Some('x') => "irregular",
        _ => "",
    }
}

/// Explicit entries fill gaps in Morpheus' stored POS (for example εἰμί is
/// "unknown" and ὅς is "adverb" despite its pronominal postags). Normalize
/// both sides so accent and spelling variants cannot defeat the promotion.
fn is_common_closed_class_lemma(lemma: &str) -> bool {
    matches!(
        normalize_form(lemma).as_str(),
        "ο" | "το" | "ειμι" | "εν" | "ος" | "αν" | "συ" | "ημεις" | "υμεις"
    )
}

fn closed_class_rank(analysis: &Analysis) -> u8 {
    if is_common_closed_class_lemma(&analysis.lemma) {
        return 0;
    }
    let stored_closed = matches!(
        analysis.pos.as_str(),
        "article" | "pronoun" | "preposition" | "particle" | "conjunction"
    );
    let postag_closed = matches!(
        analysis.postag.chars().next(),
        Some('l' | 'p' | 'r' | 'g' | 'c')
    );
    if stored_closed || postag_closed {
        1
    } else {
        2
    }
}

pub struct MorphologyDb;

impl MorphologyDb {
    pub fn new() -> Self {
        Self
    }

    /// Look up a surface form and return distinct lemma/gloss candidates.
    /// Exact matches (after grave-to-acute restoration) are preferred to the
    /// accent-stripped fallback. Candidates are stably promoted by a direct
    /// closed-class headword match, the small common closed-class allowlist,
    /// then other closed-class POS; database order is retained within each
    /// tier. Homograph rows with different glosses stay distinct, while
    /// identical headword/definition/POS rows are deduped.
    pub fn lookup(&self, conn: &Connection, form: &str) -> Vec<Analysis> {
        let exact_form = grave_to_acute(form);
        let normalized_form = normalize_form(form);
        let mut results = Self::query(conn, "f.form", &exact_form, 1.0);
        if results.is_empty() {
            results = Self::query(conn, "f.form_normalized", &normalized_form, 0.8);
        }

        results.sort_by_key(|analysis| {
            let closed_rank = closed_class_rank(analysis);
            let direct_closed_headword =
                closed_rank <= 1 && normalize_form(&analysis.lemma) == normalized_form;
            (u8::from(!direct_closed_headword), closed_rank)
        });
        let mut seen = HashSet::new();
        results.retain(|analysis| {
            seen.insert((
                analysis.lemma.clone(),
                analysis.definition.clone(),
                analysis.pos.clone(),
            ))
        });
        results
    }

    fn query(conn: &Connection, column: &str, value: &str, confidence: f32) -> Vec<Analysis> {
        let sql = format!(
            "SELECT f.lemma_id, f.postag, l.headword, l.definition, l.pos
             FROM morph.forms f
             JOIN morph.lemmas l ON f.lemma_id = l.id
             WHERE {column} = ?1
             ORDER BY f.rowid ASC"
        );
        let Ok(mut stmt) = conn.prepare(&sql) else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([value], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        }) else {
            return Vec::new();
        };

        rows.flatten()
            .map(|(lemma_id, postag, headword, definition, stored_pos)| {
                let parsed_pos = postag_pos(&postag);
                let pos = stored_pos
                    .filter(|pos| !pos.is_empty() && pos != "unknown")
                    .unwrap_or_else(|| parsed_pos.to_string());
                // A pronominal postag is more reliable than the known bad
                // stored "adverb" label on ὅς.
                let pos = if postag.starts_with('p') && pos == "adverb" {
                    "pronoun".to_string()
                } else {
                    pos
                };
                Analysis {
                    lemma: headword,
                    lemma_id,
                    postag: postag.clone(),
                    postag_human: Self::parse_postag(&postag),
                    definition,
                    confidence,
                    pos,
                }
            })
            .collect()
    }

    /// Parse a 9-position AGDT postag into human-readable string.
    ///
    /// Positions:
    /// 1. Part of speech
    /// 2. Person
    /// 3. Number
    /// 4. Tense
    /// 5. Mood
    /// 6. Voice
    /// 7. Gender
    /// 8. Case
    /// 9. Degree
    pub fn parse_postag(postag: &str) -> String {
        let chars: Vec<char> = postag.chars().collect();
        let mut parts = Vec::new();

        if let Some(&pos) = chars.first() {
            let pos_str = match pos {
                'n' => "noun",
                'v' => "verb",
                // Morpheus uses a dedicated `t` prefix for participles;
                // their other positions follow the usual verbal layout.
                't' => "participle",
                'a' => "adjective",
                'd' => "adverb",
                'l' => "article",
                'p' => "pronoun",
                'c' => "conjunction",
                'r' => "preposition",
                'e' => "exclamation",
                'm' => "numeral",
                'g' => "particle",
                'x' => "irregular",
                _ => "",
            };
            if !pos_str.is_empty() {
                parts.push(pos_str.to_string());
            }
        }

        // Person (position 2)
        if let Some(&person) = chars.get(1) {
            match person {
                '1' => parts.push("1st".to_string()),
                '2' => parts.push("2nd".to_string()),
                '3' => parts.push("3rd".to_string()),
                _ => {}
            }
        }

        // Number (position 3)
        if let Some(&number) = chars.get(2) {
            let num_str = match number {
                's' => "singular",
                'p' => "plural",
                'd' => "dual",
                _ => "",
            };
            if !num_str.is_empty() {
                parts.push(num_str.to_string());
            }
        }

        // Tense (position 4)
        if let Some(&tense) = chars.get(3) {
            let tense_str = match tense {
                'p' => "present",
                'i' => "imperfect",
                'f' => "future",
                'a' => "aorist",
                'r' => "perfect",
                'l' => "pluperfect",
                't' => "future perfect",
                _ => "",
            };
            if !tense_str.is_empty() {
                parts.push(tense_str.to_string());
            }
        }

        // Mood (position 5)
        if let Some(&mood) = chars.get(4) {
            let mood_str = match mood {
                'i' => "indicative",
                's' => "subjunctive",
                'o' => "optative",
                'n' => "infinitive",
                'p' => "participle",
                'm' => "imperative",
                _ => "",
            };
            if !mood_str.is_empty() {
                parts.push(mood_str.to_string());
            }
        }

        // Voice (position 6)
        if let Some(&voice) = chars.get(5) {
            let voice_str = match voice {
                'a' => "active",
                'm' => "middle",
                'p' => "passive",
                'e' => "medio-passive",
                _ => "",
            };
            if !voice_str.is_empty() {
                parts.push(voice_str.to_string());
            }
        }

        // Gender (position 7)
        if let Some(&gender) = chars.get(6) {
            let gender_str = match gender {
                'm' => "masculine",
                'f' => "feminine",
                'n' => "neuter",
                _ => "",
            };
            if !gender_str.is_empty() {
                parts.push(gender_str.to_string());
            }
        }

        // Case (position 8)
        if let Some(&case) = chars.get(7) {
            let case_str = match case {
                'n' => "nominative",
                'g' => "genitive",
                'd' => "dative",
                'a' => "accusative",
                'v' => "vocative",
                _ => "",
            };
            if !case_str.is_empty() {
                parts.push(case_str.to_string());
            }
        }

        // Degree (position 9)
        if let Some(&degree) = chars.get(8) {
            let degree_str = match degree {
                'c' => "comparative",
                's' => "superlative",
                _ => "",
            };
            if !degree_str.is_empty() {
                parts.push(degree_str.to_string());
            }
        }

        parts.join(" ")
    }
}

impl Default for MorphologyDb {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_postag_verb() {
        // v3spia--- = verb, 3rd person, singular, present, indicative, active
        let result = MorphologyDb::parse_postag("v3spia---");
        assert_eq!(result, "verb 3rd singular present indicative active");
    }

    #[test]
    fn test_parse_postag_noun() {
        // n-s---mn- = noun, singular, masculine, nominative
        let result = MorphologyDb::parse_postag("n-s---mn-");
        assert_eq!(result, "noun singular masculine nominative");
    }

    #[test]
    fn test_parse_postag_article() {
        // l-s---mn- = article, singular, masculine, nominative
        let result = MorphologyDb::parse_postag("l-s---mn-");
        assert_eq!(result, "article singular masculine nominative");
    }

    #[test]
    fn test_parse_postag_participle() {
        // v-sppa-n- = verb, singular, present, participle, active, nominative
        // Actually let's correct: v-sppamn-
        let result = MorphologyDb::parse_postag("v-sppamn-");
        assert_eq!(
            result,
            "verb singular present participle active masculine nominative"
        );
    }

    #[test]
    fn test_parse_postag_morpheus_participle() {
        // Shipped Morpheus analysis for ἀρξάμενος, not a synthetic v...p tag.
        let result = MorphologyDb::parse_postag("t-sa-mmn-");
        assert_eq!(
            result,
            "participle singular aorist middle masculine nominative"
        );
    }

    #[test]
    fn test_parse_postag_empty() {
        let result = MorphologyDb::parse_postag("---------");
        assert_eq!(result, "");
    }

    #[test]
    fn test_parse_postag_adjective_superlative() {
        // a-s---mns = adjective, singular, masculine, nominative, superlative
        let result = MorphologyDb::parse_postag("a-s---mns");
        assert_eq!(
            result,
            "adjective singular masculine nominative superlative"
        );
    }

    #[test]
    fn grave_restoration_covers_precomposed_decomposed_and_uppercase() {
        assert_eq!(grave_to_acute("τὴν"), "τήν");
        assert_eq!(grave_to_acute("ὃς"), "ὅς");
        assert_eq!(grave_to_acute("ΤῊΝ"), "ΤΉΝ");
        assert_eq!(grave_to_acute("ο\u{0300}"), "ό");
        assert_eq!(grave_to_acute("λέγει"), "λέγει");
        assert_eq!(normalize_form("ΛΈΓΕΙ"), "λεγει");
    }

    fn fixture_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "ATTACH DATABASE ':memory:' AS morph;
             CREATE TABLE morph.lemmas (
                 id INTEGER PRIMARY KEY,
                 headword TEXT NOT NULL,
                 definition TEXT,
                 pos TEXT
             );
             CREATE TABLE morph.forms (
                 form TEXT NOT NULL,
                 form_normalized TEXT NOT NULL,
                 lemma_id INTEGER NOT NULL,
                 postag TEXT NOT NULL
             );
             INSERT INTO morph.lemmas VALUES
                 (1, 'ἄτη', 'bewilderment', 'noun'),
                 (2, 'ὁ', 'the', 'article'),
                 (3, 'ὅσος', 'quantus', 'adjective'),
                 (4, 'ὅς', 'who, which', 'adverb'),
                 (5, 'λέγω', 'say, speak', 'unknown'),
                 (6, 'εἰμί', 'to be', 'unknown'),
                 (7, 'εἰς', 'into', 'preposition'),
                 (8, 'ἐν', 'in', 'preposition'),
                 (9, 'ἐάν', 'if haply', 'conjunction'),
                 (10, 'λέγω', 'say, speak', 'unknown'),
                 (11, 'ἤν', 'see! see there!', 'exclamation');
             INSERT INTO morph.forms VALUES
                 ('’τήν', 'την', 1, 'n-s---fa-'),
                 ('τήν', 'την', 2, 'l-s---fa-'),
                 ('ὅς’', 'ος', 3, 'a-s---mn-'),
                 ('ὅς', 'ος', 4, 'p-s---mn-'),
                 ('λέγει', 'λεγει', 5, 'v2spie---'),
                 ('λέγει', 'λεγει', 5, 'v3spia---'),
                 ('λέγει', 'λεγει', 10, 'v3spia---'),
                 ('ἐν', 'εν', 6, 'v3piia---'),
                 ('ἐν', 'εν', 7, 'r--------'),
                 ('ἐν', 'εν', 8, 'r--------'),
                 ('ἦν', 'ην', 9, 'c--------'),
                 ('ἦν', 'ην', 6, 'v3siia---'),
                 ('ἦν', 'ην', 11, 'e--------');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn grave_forms_use_exact_rows_instead_of_accent_stripped_homographs() {
        let conn = fixture_db();
        let db = MorphologyDb::new();
        for (form, expected) in [("τὴν", "ὁ"), ("ὃς", "ὅς"), ("λέγει", "λέγω")]
        {
            let results = db.lookup(&conn, form);
            assert_eq!(results[0].lemma, expected, "{form}");
            assert!(results.iter().all(|analysis| analysis.confidence == 1.0));
        }
        assert_eq!(db.lookup(&conn, "τὴν").len(), 1);
        assert_eq!(db.lookup(&conn, "ὃς")[0].pos, "pronoun");
    }

    #[test]
    fn direct_headword_and_common_closed_class_candidates_rank_first() {
        let conn = fixture_db();
        let db = MorphologyDb::new();
        let en = db.lookup(&conn, "ἐν");
        assert_eq!(
            en.iter().map(|a| a.lemma.as_str()).collect::<Vec<_>>(),
            ["ἐν", "εἰμί", "εἰς"]
        );
        let een = db.lookup(&conn, "ἦν");
        assert_eq!(een[0].lemma, "εἰμί");
        assert_eq!(een[1].lemma, "ἐάν");
        assert_eq!(een[2].lemma, "ἤν");
    }

    #[test]
    fn duplicate_homograph_glosses_are_collapsed_but_parse_rows_remain_exact() {
        let conn = fixture_db();
        let results = MorphologyDb::new().lookup(&conn, "λέγει");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].definition, "say, speak");
        assert_eq!(results[0].pos, "verb");
        assert_eq!(results[0].postag, "v2spie---");
    }

    #[test]
    fn accent_stripped_uppercase_fallback_still_works() {
        let conn = fixture_db();
        let results = MorphologyDb::new().lookup(&conn, "ΛΕΓΕΙ");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].lemma, "λέγω");
        assert_eq!(results[0].confidence, 0.8);
    }
}
