use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Normalize a Greek surface form to match the DB's accent-stripped,
/// lowercased `form_normalized` column: NFD-decompose, drop combining
/// diacritical marks (Unicode category Mn), then lowercase and recompose.
fn normalize_form(form: &str) -> String {
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
}

pub struct MorphologyDb;

impl MorphologyDb {
    pub fn new() -> Self {
        Self
    }

    /// Look up a surface form. Returns all analyses, exact matches first.
    /// Queries the attached 'morph' database.
    pub fn lookup(&self, conn: &Connection, form: &str) -> Vec<Analysis> {
        let mut results = Vec::new();

        // First try exact match
        if let Ok(mut stmt) = conn.prepare(
            "SELECT f.lemma_id, f.postag, l.headword, l.definition
             FROM morph.forms f
             JOIN morph.lemmas l ON f.lemma_id = l.id
             WHERE f.form = ?1",
        ) {
            if let Ok(rows) = stmt.query_map([form], |row| {
                let lemma_id: i64 = row.get(0)?;
                let postag: String = row.get(1)?;
                let headword: String = row.get(2)?;
                let definition: String = row.get(3)?;
                Ok((lemma_id, postag, headword, definition))
            }) {
                for row in rows.flatten() {
                    let (lemma_id, postag, headword, definition) = row;
                    let postag_human = Self::parse_postag(&postag);
                    results.push(Analysis {
                        lemma: headword,
                        lemma_id,
                        postag: postag.clone(),
                        postag_human,
                        definition,
                        confidence: 1.0,
                    });
                }
            }
        }

        // If no exact results, try normalized form
        if results.is_empty() {
            let normalized: String = normalize_form(form);

            if let Ok(mut stmt) = conn.prepare(
                "SELECT f.lemma_id, f.postag, l.headword, l.definition
                 FROM morph.forms f
                 JOIN morph.lemmas l ON f.lemma_id = l.id
                 WHERE f.form_normalized = ?1",
            ) {
                if let Ok(rows) = stmt.query_map([&normalized], |row| {
                    let lemma_id: i64 = row.get(0)?;
                    let postag: String = row.get(1)?;
                    let headword: String = row.get(2)?;
                    let definition: String = row.get(3)?;
                    Ok((lemma_id, postag, headword, definition))
                }) {
                    for row in rows.flatten() {
                        let (lemma_id, postag, headword, definition) = row;
                        let postag_human = Self::parse_postag(&postag);
                        results.push(Analysis {
                            lemma: headword,
                            lemma_id,
                            postag: postag.clone(),
                            postag_human,
                            definition,
                            confidence: 0.8,
                        });
                    }
                }
            }
        }

        results
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
}
