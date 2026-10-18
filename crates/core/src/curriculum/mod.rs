use crate::db::Database;
use crate::srs::Card;
use serde::{Deserialize, Serialize};

/// A card counts as mastered once its memory-state stability reaches this
/// many days (i.e. it is well entrenched in long-term memory). A card that
/// has merely been seen once does NOT count.
pub const MASTERY_STABILITY_DAYS: f32 = 30.0;

/// Fraction of a chapter's cards that must be mastered before the next
/// chapter's unlock requirement counts as met.
pub const UNLOCK_THRESHOLD: f32 = 0.8;

/// Number of mastered cards required in a chapter before the next chapter's
/// unlock requirement counts as met: `ceil(UNLOCK_THRESHOLD * card_count)`.
/// A card_count of 0 yields 0, so the requirement is always met — this
/// prevents deadlocks for chapters past card coverage (e.g. Athenaze ch.17+).
fn needed_for_unlock(card_count: i32) -> i32 {
    (UNLOCK_THRESHOLD * card_count as f32).ceil() as i32
}

// --- Alphabet gate ---

/// Number of questions in the Alphabet Check.
pub const ALPHABET_CHECK_N: u32 = 24;
/// Minimum correct answers needed to pass the Alphabet Check.
pub const ALPHABET_CHECK_PASS_MIN: u32 = 22;

/// Letters-track lane thresholds for the organic (no-button) unlock: the
/// learner must have measured letters-practice accuracy of at least 85%
/// over at least 20 attempts, spread across at least 2 distinct UTC days.
const ORGANIC_MIN_PRACTICE: u32 = 20;
const ORGANIC_MIN_ACCURACY: f64 = 0.85;
const ORGANIC_MIN_DAYS: u32 = 2;

/// A letters-track card counts as "familiar" (toward the intra-letters
/// unlock progress) once it has been reviewed at least this many times,
/// regardless of source — vocabulary chapters keep the 30-day stability
/// rule instead.
const LETTERS_FAMILIAR_REPS: i32 = 2;

/// How the alphabet gate was satisfied. Persisted in `user_settings` under
/// its serde snake-case name (e.g. `"check_passed"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphabetGateMethod {
    CheckPassed,
    Organic,
    DeclaredKnown,
    VocabHistory,
}

impl AlphabetGateMethod {
    /// Serde name as stored in the `user_settings.alphabet_gate` value
    /// (identical to the `rename_all = "snake_case"` serialized form; a unit
    /// test pins the two together).
    pub fn name(&self) -> &'static str {
        match self {
            Self::CheckPassed => "check_passed",
            Self::Organic => "organic",
            Self::DeclaredKnown => "declared_known",
            Self::VocabHistory => "vocab_history",
        }
    }

    /// Inverse of [`AlphabetGateMethod::name`]; unknown strings -> None.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "check_passed" => Some(Self::CheckPassed),
            "organic" => Some(Self::Organic),
            "declared_known" => Some(Self::DeclaredKnown),
            "vocab_history" => Some(Self::VocabHistory),
            _ => None,
        }
    }
}

/// Snapshot of the alphabet-gate evaluation for progress UIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlphabetGateState {
    pub satisfied: bool,
    pub method: Option<AlphabetGateMethod>,
    pub letters_introduced: u32,
    pub letters_total: u32,
    pub letters_practice_total: u32,
    pub letters_practice_correct: u32,
    pub letters_active_days: u32,
    pub letters_chapters_met: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurriculumInfo {
    pub textbook: String,
    pub display_name: String,
    pub total_chapters: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterInfo {
    pub chapter: i32,
    pub title: String,
    pub card_count: i32,
    pub mastered_count: i32,
    pub unlocked: bool,
    /// Whether this chapter has loaded card data. Chapters with no cards
    /// (card_count == 0) are marked unavailable so the UI can render them
    /// as "Coming Soon".
    pub available: bool,
    /// Unlock-gating hint shown for locked chapters whose previous chapter
    /// is not yet mastered enough. Empty when the chapter is already
    /// unlocked (including default-unlocked chapters) or when the unlock
    /// requirement is met and the user may simply tap to unlock.
    pub unlock_hint: String,
}

/// Returns true if a card belongs to the given curriculum textbook.
/// Matches an exact tag (e.g. "jact") or a volume-prefixed tag
/// (e.g. "athenaze-i"/"athenaze-ii" for the "athenaze" textbook,
/// "dcc-core" for the "dcc" textbook). Using an exact/prefix match
/// prevents volumes from cross-contaminating each other.
pub fn card_in_textbook(card: &Card, textbook: &str) -> bool {
    let prefix = format!("{}-", textbook);
    card.tags
        .iter()
        .any(|t| t == textbook || t.starts_with(&prefix))
}

/// Card ids from `memory_states` matching a simple `WHERE <sql> < ?1` filter
/// (e.g. `"stability >= ?1"` for mastery, `"reps >= ?1"` for letters
/// familiarity). Returns an empty set on any query error.
fn memory_ids_where(db: &Database, where_sql: &str, threshold: f64) -> std::collections::HashSet<String> {
    db.conn()
        .prepare(&format!("SELECT card_id FROM memory_states WHERE {where_sql}"))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([threshold], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
}

pub struct Curriculum;

impl Curriculum {
    /// Get available textbooks
    pub fn get_textbooks() -> Vec<CurriculumInfo> {
        vec![
            // Stage 0: alphabet + Erasmian pronunciation. Ordered FIRST so
            // that onboarding/new-card fill starts with letters before any
            // vocabulary textbook.
            CurriculumInfo {
                textbook: "letters".to_string(),
                display_name: "The Alphabet & Pronunciation".to_string(),
                total_chapters: 5,
            },
            CurriculumInfo {
                textbook: "athenaze".to_string(),
                display_name: "Athenaze: An Introduction to Ancient Greek".to_string(),
                total_chapters: 31,
            },
            CurriculumInfo {
                textbook: "jact".to_string(),
                display_name: "Reading Greek (JACT)".to_string(),
                total_chapters: 20,
            },
            CurriculumInfo {
                textbook: "dcc".to_string(),
                display_name: "DCC Core Vocabulary".to_string(),
                // The 550 DCC-Core cards are a single frequency-ordered list,
                // exposed as one pseudo-chapter (chapter 0).
                total_chapters: 1,
            },
        ]
    }

    /// Evaluate the alphabet gate: is vocabulary *new-card introduction*
    /// unlocked? (Due reviews are never gated.)
    ///
    /// DEVIATION from DESIGN-alphabet-gate.md §4.1: the signature takes the
    /// full loaded card slice (`cards`) instead of a bare `letters_total`;
    /// `letters_total` is computed internally, because the letters-chapter
    /// exposure counts also come from the cards. Behavior is the design's.
    ///
    /// Evaluation order:
    /// 1. fail-open when the letters track isn't loaded (`letters_total == 0`),
    ///    so the gate can never brick a build;
    /// 2. the persisted latch (`user_settings.alphabet_gate`) — once latched,
    ///    the gate never re-locks;
    /// 3. Lane C grandfather: any non-letters `memory_states` row exists;
    /// 4. Lane B organic: full letters coverage AND >=85% measured letters
    ///    practice accuracy over >=20 attempts AND activity on >=2 distinct
    ///    UTC days.
    pub fn alphabet_gate_state(db: &Database, cards: &[Card]) -> AlphabetGateState {
        let letters_total = cards
            .iter()
            .filter(|c| card_in_textbook(c, "letters"))
            .count() as u32;

        let letters_introduced: u32 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM memory_states WHERE card_id LIKE 'letters-%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as u32;

        let (letters_practice_total, letters_practice_correct): (u32, u32) = db
            .conn()
            .query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(CASE WHEN rating >= 3 THEN 1 ELSE 0 END), 0)
                 FROM review_logs
                 WHERE source = 'practice' AND card_id LIKE 'letters-%'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .map(|(total, correct)| (total as u32, correct as u32))
            .unwrap_or((0, 0));

        let letters_active_days: u32 = db
            .conn()
            .query_row(
                "SELECT COUNT(DISTINCT substr(timestamp, 1, 10))
                 FROM review_logs WHERE card_id LIKE 'letters-%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as u32;

        // Letters chapters where >=80% of the seeded cards have been seen
        // twice (reps >= 2, any source): the letters-track exposure bar.
        let letters_chapters_met = {
            let reps2_ids = memory_ids_where(db, "reps >= ?1", LETTERS_FAMILIAR_REPS as f64);
            cards
                .iter()
                .filter(|c| card_in_textbook(c, "letters"))
                .fold(std::collections::BTreeMap::<i32, (u32, u32)>::new(), |mut acc, c| {
                    let entry = acc.entry(c.chapter).or_default();
                    entry.0 += 1;
                    if reps2_ids.contains(&c.id) {
                        entry.1 += 1;
                    }
                    acc
                })
                .values()
                .filter(|&&(total, met)| {
                    total > 0 && (met as i32) >= needed_for_unlock(total as i32)
                })
                .count() as u32
        };

        // 1. Fail-open when the letters deck isn't loaded.
        if letters_total == 0 {
            return AlphabetGateState {
                satisfied: true,
                method: None,
                letters_introduced,
                letters_total,
                letters_practice_total,
                letters_practice_correct,
                letters_active_days,
                letters_chapters_met,
            };
        }

        // 2. The latch: once written, the gate stays satisfied forever.
        let latched: Option<AlphabetGateMethod> = db
            .conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = 'alphabet_gate'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| AlphabetGateMethod::from_name(&v));
        if let Some(method) = latched {
            return AlphabetGateState {
                satisfied: true,
                method: Some(method),
                letters_introduced,
                letters_total,
                letters_practice_total,
                letters_practice_correct,
                letters_active_days,
                letters_chapters_met,
            };
        }

        // 3. Lane C grandfather: any vocabulary/lookup memory state means the
        //    learner had a pre-gate vocabulary habit; auto-satisfy.
        let has_vocab_history: bool = db
            .conn()
            .query_row(
                "SELECT 1 FROM memory_states WHERE card_id NOT LIKE 'letters-%' LIMIT 1",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if has_vocab_history {
            return AlphabetGateState {
                satisfied: true,
                method: Some(AlphabetGateMethod::VocabHistory),
                letters_introduced,
                letters_total,
                letters_practice_total,
                letters_practice_correct,
                letters_active_days,
                letters_chapters_met,
            };
        }

        // 4. Lane B organic.
        let organic = letters_introduced >= letters_total
            && letters_practice_total >= ORGANIC_MIN_PRACTICE
            && (letters_practice_correct as f64)
                >= ORGANIC_MIN_ACCURACY * letters_practice_total as f64
            && letters_active_days >= ORGANIC_MIN_DAYS;
        let method = organic.then_some(AlphabetGateMethod::Organic);

        AlphabetGateState {
            satisfied: organic,
            method,
            letters_introduced,
            letters_total,
            letters_practice_total,
            letters_practice_correct,
            letters_active_days,
            letters_chapters_met,
        }
    }

    /// Persist the alphabet-gate latch: `user_settings.alphabet_gate` = the
    /// method's serde snake-case name, `alphabet_gate_at` = now (ISO-8601).
    /// Once written it is never re-locked.
    pub fn record_alphabet_gate(db: &Database, method: AlphabetGateMethod) -> anyhow::Result<()> {
        let now = crate::srs::now_iso8601();
        // Routed through setting_put so the latch is HLC-stamped and dual-
        // written to settings_sync + user_settings (participates in sync;
        // local readers of user_settings are unaffected).
        crate::sync::setting_put(db, "alphabet_gate", method.name())?;
        crate::sync::setting_put(db, "alphabet_gate_at", &now)?;
        Ok(())
    }

    /// Alphabet Check verdict: pass at >=22 of >=24.
    pub fn evaluate_alphabet_check(correct: u32, total: u32) -> bool {
        correct >= ALPHABET_CHECK_PASS_MIN && total >= ALPHABET_CHECK_N
    }


    ///
    /// `cards` is the full set of loaded flashcards; per-chapter `card_count`
    /// and `mastered_count` are computed from it (mastered = has a
    /// memory_states row with stability >= MASTERY_STABILITY_DAYS). Chapters
    /// with no cards are marked `available == false` so the UI can render
    /// them as "Coming Soon".
    pub fn get_chapters(db: &Database, textbook: &str, cards: &[Card]) -> Vec<ChapterInfo> {
        let info = Self::get_textbooks();
        let total_chapters = info
            .iter()
            .find(|t| t.textbook == textbook)
            .map(|t| t.total_chapters)
            .unwrap_or(0);

        // Set of card ids that count as "mastered". Vocabulary textbooks use
        // the long-term rule (stability >= 30 days); the letters track counts
        // a card as soon as it has been seen twice (reps >= 2, any source),
        // because letter familiarity is an exposure-level skill and the 30d
        // rule would take weeks to unlock intra-letters chapters.
        let mastered_ids: std::collections::HashSet<String> = if textbook == "letters" {
            memory_ids_where(db, "reps >= ?1", LETTERS_FAMILIAR_REPS as f64)
        } else {
            memory_ids_where(db, "stability >= ?1", MASTERY_STABILITY_DAYS as f64)
        };

        // Alphabet gate: while unmet, vocabulary tracks are presented fully
        // locked regardless of the stored progress rows.
        let gate_locked =
            textbook != "letters" && !Self::alphabet_gate_state(db, cards).satisfied;

        // Cards belonging to this textbook, so we only scan them once per chapter.
        let textbook_cards: Vec<&Card> =
            cards.iter().filter(|c| card_in_textbook(c, textbook)).collect();

        let mut chapters = Vec::new();

        // The DCC pseudo-chapter uses chapter number 0; other textbooks are 1-based.
        let chapter_range: Vec<i32> = if textbook == "dcc" {
            vec![0]
        } else {
            (1..=total_chapters).collect()
        };

        // Default (always-unlocked) chapter of this textbook: ch.1 for
        // 1-based tracks, ch.0 for DCC — used for the gate hint below.
        let default_chapter: i32 = if textbook == "dcc" { 0 } else { 1 };

        for chapter in chapter_range {
            // Check if chapter is unlocked
            let unlocked: bool = db
                .conn()
                .query_row(
                    "SELECT unlocked FROM curriculum_progress WHERE textbook = ?1 AND chapter = ?2",
                    rusqlite::params![textbook, chapter],
                    |row| row.get::<_, i32>(0).map(|v| v != 0),
                )
                .unwrap_or(chapter <= 1); // Chapter 1 (and the DCC chapter 0) unlocked by default
            // The alphabet gate overrides: vocabulary chapters report locked
            // until the gate is satisfied.
            let unlocked = unlocked && !gate_locked;

            let chapter_cards: Vec<&&Card> = textbook_cards
                .iter()
                .filter(|c| c.chapter == chapter)
                .collect();

            let card_count = chapter_cards.len() as i32;
            let mastered_count = chapter_cards
                .iter()
                .filter(|c| mastered_ids.contains(&c.id))
                .count() as i32;
            let available = card_count > 0;

            // Unlock-gating hint. Empty when the chapter is already unlocked
            // (including the default-unlocked first chapter: athenaze/jact
            // ch.1, dcc ch.0). For a locked chapter, the requirement is met
            // when the previous chapter's mastered count reaches
            // needed_for_unlock(prev card_count). A previous chapter with no
            // cards at all (past card coverage, e.g. Athenaze ch.17+) counts
            // as met so later chapters can never deadlock.
            let unlock_hint = if unlocked {
                String::new()
            } else if gate_locked && chapter == default_chapter {
                "Finish 'The Alphabet & Pronunciation' — or pass the Alphabet Check."
                    .to_string()
            } else {
                let prev = chapter - 1;
                let prev_count = textbook_cards
                    .iter()
                    .filter(|c| c.chapter == prev)
                    .count() as i32;
                let prev_mastered = textbook_cards
                    .iter()
                    .filter(|c| c.chapter == prev && mastered_ids.contains(&c.id))
                    .count() as i32;
                let needed = needed_for_unlock(prev_count);
                if prev_mastered >= needed {
                    String::new()
                } else {
                    format!(
                        "Master 80% of Chapter {} to unlock ({}/{})",
                        prev, prev_mastered, needed
                    )
                }
            };

            let title = chapter_title(textbook, chapter);

            chapters.push(ChapterInfo {
                chapter,
                title,
                card_count,
                mastered_count,
                unlocked,
                available,
                unlock_hint,
            });
        }

        chapters
    }

    /// Unlock a chapter for study
    pub fn unlock_chapter(db: &Database, textbook: &str, chapter: i32) -> anyhow::Result<()> {
        db.conn().execute(
            "INSERT INTO curriculum_progress (textbook, chapter, unlocked)
             VALUES (?1, ?2, 1)
             ON CONFLICT(textbook, chapter) DO UPDATE SET unlocked = 1",
            rusqlite::params![textbook, chapter],
        )?;
        Ok(())
    }
}

/// Get the title for a given chapter. Returns placeholder titles for now.
fn chapter_title(textbook: &str, chapter: i32) -> String {
    match textbook {
        "letters" => match chapter {
            1 => "The Vowels".to_string(),
            2 => "The Stops".to_string(),
            3 => "The Continuants".to_string(),
            4 => "Breathings & Accents".to_string(),
            5 => "Diphthongs".to_string(),
            _ => format!("Chapter {}", chapter),
        },
        "athenaze" => match chapter {
            1 => "Ο ΔΙΚΑΙΟΠΟΛΙΣ (Dicaeopolis)".to_string(),
            2 => "Ο ΞΑΝΘΙΑΣ (Xanthias)".to_string(),
            3 => "Ο ΑΡΟΤΟΣ (The Plowing)".to_string(),
            4 => "ΠΡΟΣ ΤΗΙ ΚΡΗΝΗΙ (At the Spring)".to_string(),
            5 => "Ο ΛΥΚΟΣ (The Wolf)".to_string(),
            6 => "Ο ΜΥΘΟΣ (The Story)".to_string(),
            7 => "Ο ΚΥΚΛΩΨ (The Cyclops)".to_string(),
            8 => "ΠΡΟΣ ΤΟ ΑΣΤΥ (To the City)".to_string(),
            9 => "Η ΠΑΝΗΓΥΡΙΣ (The Festival)".to_string(),
            10 => "Η ΣΥΜΦΟΡΑ (The Misfortune)".to_string(),
            _ => format!("Chapter {}", chapter),
        },
        "jact" => match chapter {
            1 => "The Insurance Fraud".to_string(),
            2 => "The Insurance Fraud (continued)".to_string(),
            3 => "Aristophanes' Acharnians".to_string(),
            _ => format!("Section {}", chapter),
        },
        "dcc" => "DCC Core Vocabulary (Frequency List)".to_string(),
        _ => format!("Chapter {}", chapter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_card(id: &str, chapter: i32, textbook_tag: &str) -> Card {
        Card {
            id: id.to_string(),
            front: String::new(),
            back: String::new(),
            chapter,
            card_type: "Vocab".to_string(),
            pos: "noun".to_string(),
            tags: vec![textbook_tag.to_string(), format!("chapter-{}", chapter)],
        }
    }

    #[test]
    fn test_get_textbooks() {
        let textbooks = Curriculum::get_textbooks();
        assert_eq!(textbooks.len(), 4);
        // Stage 0 letters track comes first (onboarding priority).
        assert_eq!(textbooks[0].textbook, "letters");
        assert_eq!(textbooks[0].total_chapters, 5);
        assert_eq!(textbooks[1].textbook, "athenaze");
        assert_eq!(textbooks[2].textbook, "jact");
        assert_eq!(textbooks[3].textbook, "dcc");
    }

    #[test]
    fn test_get_chapters() {
        let db = Database::open(":memory:", None).unwrap();
        let chapters = Curriculum::get_chapters(&db, "athenaze", &[]);
        assert_eq!(chapters.len(), 31);
        // Chapter 1 should be unlocked by default
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].title, "Ο ΔΙΚΑΙΟΠΟΛΙΣ (Dicaeopolis)");
    }

    #[test]
    fn test_get_chapters_card_counts_and_availability() {
        let db = Database::open(":memory:", None).unwrap();
        let cards = vec![
            sample_card("ath1-001", 1, "athenaze-i"),
            sample_card("ath1-002", 1, "athenaze-i"),
            sample_card("ath17-001", 17, "athenaze-ii"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        // Chapter 1 has 2 cards, is available; chapter 26 has none, not available.
        assert_eq!(chapters[0].card_count, 2);
        assert!(chapters[0].available);
        assert_eq!(chapters[16].card_count, 1); // chapter 17
        assert!(chapters[16].available);
        assert_eq!(chapters[25].card_count, 0); // chapter 26
        assert!(!chapters[25].available);
    }

    #[test]
    fn test_get_chapters_mastered_count() {
        let db = Database::open(":memory:", None).unwrap();
        // ath1-001 is mastered (stability >= 30); ath1-002 has been reviewed
        // (reps >= 1) but at low stability, so it must NOT count as mastered.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-001', 30.0, 5.0, '2024-01-01T00:00:00Z', 3, 0),
                        ('ath1-002', 5.0, 5.0, '2024-01-01T00:00:00Z', 3, 0)",
                [],
            )
            .unwrap();
        let cards = vec![
            sample_card("ath1-001", 1, "athenaze-i"),
            sample_card("ath1-002", 1, "athenaze-i"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert_eq!(chapters[0].card_count, 2);
        assert_eq!(chapters[0].mastered_count, 1);
    }

    #[test]
    fn test_mastery_stability_threshold() {
        let db = Database::open(":memory:", None).unwrap();
        // 29.9 days: below threshold -> not mastered.
        // 30.0 days: at threshold -> mastered.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-low', 29.9, 5.0, '2024-01-01T00:00:00Z', 5, 0),
                        ('ath1-high', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                [],
            )
            .unwrap();
        let cards = vec![
            sample_card("ath1-low", 1, "athenaze-i"),
            sample_card("ath1-high", 1, "athenaze-i"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert_eq!(chapters[0].card_count, 2);
        assert_eq!(chapters[0].mastered_count, 1);
    }

    #[test]
    fn test_needed_for_unlock_ceil() {
        // 80% of 5 rounds up to 4.
        assert_eq!(needed_for_unlock(5), 4);
        assert_eq!(needed_for_unlock(4), 4); // ceil(3.2)
        assert_eq!(needed_for_unlock(10), 8);
        assert_eq!(needed_for_unlock(1), 1);
        // 0 cards -> 0 needed, i.e. requirement always met (no deadlock).
        assert_eq!(needed_for_unlock(0), 0);
    }

    #[test]
    fn test_unlock_hint_locked_chapter() {
        let db = Database::open(":memory:", None).unwrap();
        // Chapter 1 has 5 cards, 2 mastered; need ceil(0.8 * 5) = 4.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-001', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0),
                        ('ath1-002', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                [],
            )
            .unwrap();
        let cards = vec![
            sample_card("ath1-001", 1, "athenaze-i"),
            sample_card("ath1-002", 1, "athenaze-i"),
            sample_card("ath1-003", 1, "athenaze-i"),
            sample_card("ath1-004", 1, "athenaze-i"),
            sample_card("ath1-005", 1, "athenaze-i"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        // Chapter 2 is locked and the requirement is not met.
        assert!(!chapters[1].unlocked);
        assert_eq!(
            chapters[1].unlock_hint,
            "Master 80% of Chapter 1 to unlock (2/4)"
        );

        // Once unlocked, the hint clears even though mastery is unchanged.
        Curriculum::unlock_chapter(&db, "athenaze", 2).unwrap();
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert!(chapters[1].unlocked);
        assert_eq!(chapters[1].unlock_hint, "");
    }

    #[test]
    fn test_unlock_hint_empty_when_requirement_met() {
        let db = Database::open(":memory:", None).unwrap();
        // Chapter 1 has 5 cards, 4 mastered = ceil(0.8 * 5): requirement met.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-001', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0),
                        ('ath1-002', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0),
                        ('ath1-003', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0),
                        ('ath1-004', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                [],
            )
            .unwrap();
        let cards = vec![
            sample_card("ath1-001", 1, "athenaze-i"),
            sample_card("ath1-002", 1, "athenaze-i"),
            sample_card("ath1-003", 1, "athenaze-i"),
            sample_card("ath1-004", 1, "athenaze-i"),
            sample_card("ath1-005", 1, "athenaze-i"),
        ];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        // Still locked, but the user may tap to unlock: no hint.
        assert!(!chapters[1].unlocked);
        assert_eq!(chapters[1].unlock_hint, "");
    }

    #[test]
    fn test_unlock_hint_empty_for_default_chapters() {
        let db = Database::open(":memory:", None).unwrap();
        // Athenaze chapter 1 is unlocked by default: hint always empty.
        let chapters = Curriculum::get_chapters(&db, "athenaze", &[]);
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
        // JACT chapter 1 likewise.
        let chapters = Curriculum::get_chapters(&db, "jact", &[]);
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
        // DCC chapter 0 is always unlocked: hint always empty.
        let cards = vec![sample_card("dcc-001", 0, "dcc-core")];
        let chapters = Curriculum::get_chapters(&db, "dcc", &cards);
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
    }

    #[test]
    fn test_unlock_hint_empty_when_previous_chapter_has_no_cards() {
        let db = Database::open(":memory:", None).unwrap();
        // Athenaze chapter 16 has no cards (past card coverage), so chapter
        // 17's requirement counts as met and must not deadlock.
        let cards = vec![sample_card("ath17-001", 17, "athenaze-ii")];
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert!(!chapters[16].unlocked); // chapter 17
        assert_eq!(chapters[16].unlock_hint, "");
    }

    #[test]
    fn test_get_chapters_dcc() {
        let db = Database::open(":memory:", None).unwrap();
        let cards = vec![sample_card("dcc-001", 0, "dcc-core")];
        let chapters = Curriculum::get_chapters(&db, "dcc", &cards);
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].chapter, 0);
        assert_eq!(chapters[0].card_count, 1);
        assert!(chapters[0].available);
        assert!(chapters[0].unlocked);
    }

    #[test]
    fn test_unlock_chapter() {
        let db = Database::open(":memory:", None).unwrap();

        // Chapter 2 should not be unlocked initially
        let chapters = Curriculum::get_chapters(&db, "athenaze", &[]);
        assert!(!chapters[1].unlocked);

        // Unlock chapter 2
        Curriculum::unlock_chapter(&db, "athenaze", 2).unwrap();

        // Now it should be unlocked
        let chapters = Curriculum::get_chapters(&db, "athenaze", &[]);
        assert!(chapters[1].unlocked);
    }

    /// A Stage-0 letters card: real cards are tagged ["letters", "chapter-N"].
    fn letters_card(id: &str, chapter: i32) -> Card {
        Card {
            id: id.to_string(),
            front: String::new(),
            back: String::new(),
            chapter,
            card_type: "Letter".to_string(),
            pos: "letter".to_string(),
            tags: vec!["letters".to_string(), format!("chapter-{}", chapter)],
        }
    }

    #[test]
    fn test_card_in_textbook_letters() {
        let card = letters_card("letters-010", 2);
        assert!(card_in_textbook(&card, "letters"));
        // ...and it does not leak into the vocabulary textbooks.
        assert!(!card_in_textbook(&card, "athenaze"));
        assert!(!card_in_textbook(&card, "jact"));
        assert!(!card_in_textbook(&card, "dcc"));
        // Vocabulary cards do not leak into the letters track either.
        assert!(!card_in_textbook(&sample_card("ath1-001", 1, "athenaze-i"), "letters"));
    }

    #[test]
    fn test_get_chapters_letters() {
        let db = Database::open(":memory:", None).unwrap();
        // Mirror data/cards/letters.json shape: 7 ch.1 cards + 1 per later
        // chapter so every chapter is available.
        let mut cards: Vec<Card> = (1..=7)
            .map(|i| letters_card(&format!("letters-{i:03}"), 1))
            .collect();
        cards.push(letters_card("letters-020", 2));
        cards.push(letters_card("letters-026", 3));
        cards.push(letters_card("letters-030", 4));
        cards.push(letters_card("letters-040", 5));

        let chapters = Curriculum::get_chapters(&db, "letters", &cards);
        assert_eq!(chapters.len(), 5);
        assert!(chapters.iter().all(|c| c.available));
        let titles: Vec<&str> = chapters.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "The Vowels",
                "The Stops",
                "The Continuants",
                "Breathings & Accents",
                "Diphthongs"
            ]
        );
        // Chapter 1 is unlocked by default, with no hint.
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
        // Chapter 2 is locked; none of the 7 ch.1 cards is mastered, and
        // ceil(0.8 * 7) = 6 are needed.
        assert!(!chapters[1].unlocked);
        assert!(chapters[1].unlock_hint.contains("Chapter 1"));
        assert_eq!(
            chapters[1].unlock_hint,
            "Master 80% of Chapter 1 to unlock (0/6)"
        );
    }

    #[test]
    fn test_letters_unlock_chain_with_mastered_chapter_1() {
        let db = Database::open(":memory:", None).unwrap();
        // Master all 7 chapter-1 letters (stability >= MASTERY_STABILITY_DAYS).
        for i in 1..=7 {
            db.conn()
                .execute(
                    "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                     VALUES (?1, 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                    [format!("letters-{i:03}")],
                )
                .unwrap();
        }
        let mut cards: Vec<Card> = (1..=7)
            .map(|i| letters_card(&format!("letters-{i:03}"), 1))
            .collect();
        cards.push(letters_card("letters-008", 2));

        let chapters = Curriculum::get_chapters(&db, "letters", &cards);
        assert_eq!(chapters[0].card_count, 7);
        assert_eq!(chapters[0].mastered_count, 7);
        // The unlock requirement is now met (7 >= 6): the hint clears and
        // the user may simply tap to unlock chapter 2.
        assert!(!chapters[1].unlocked);
        assert_eq!(chapters[1].unlock_hint, "");

        Curriculum::unlock_chapter(&db, "letters", 2).unwrap();
        let chapters = Curriculum::get_chapters(&db, "letters", &cards);
        assert!(chapters[1].unlocked);
    }

    // --- Alphabet gate tests (DESIGN-alphabet-gate.md §8) ---

    /// The 43 letters cards mirroring `data/cards/letters.json` chapter
    /// layout (7 vowels, 9 stops, 9 continuants, 7 diacritics, 11 diphthongs).
    fn letters_deck() -> Vec<Card> {
        (1..=43)
            .map(|i| {
                let chapter = match i {
                    1..=7 => 1,
                    8..=16 => 2,
                    17..=25 => 3,
                    26..=32 => 4,
                    _ => 5,
                };
                letters_card(&format!("letters-{i:03}"), chapter)
            })
            .collect()
    }

    /// Full letters coverage (43 memory_states rows) plus `correct + wrong`
    /// letters practice rows spread across `dates`.
    fn seed_organic_lane(db: &Database, dates: &[&str], correct: usize, wrong: usize) {
        for i in 1..=43 {
            db.conn()
                .execute(
                    "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                     VALUES (?1, 30.0, 5.0, '2024-01-01T00:00:00Z', 1, 0)",
                    [format!("letters-{i:03}")],
                )
                .unwrap();
        }
        for k in 0..(correct + wrong) {
            let rating = if k < wrong { 1 } else { 4 };
            let ts = format!("{}T10:{:02}:00Z", dates[k % dates.len()], k % 60);
            db.conn()
                .execute(
                    "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days, source, uuid, device_id, hlc)
                     VALUES (?1, ?2, ?3, 0.0, 0.0, 'practice', ?4, ?5, ?6)",
                    rusqlite::params![
                        format!("letters-{:03}", (k % 43) + 1),
                        ts,
                        rating,
                        uuid::Uuid::new_v4().to_string(),
                        crate::sync::device_id(db),
                        crate::sync::hlc_now(db).to_string(),
                    ],
                )
                .unwrap();
        }
    }

    #[test]
    fn gate_fresh_db_unsatisfied() {
        let db = Database::open(":memory:", None).unwrap();
        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(!state.satisfied);
        assert_eq!(state.method, None);
        assert_eq!(state.letters_total, 43);
        assert_eq!(state.letters_introduced, 0);
        assert_eq!(state.letters_practice_total, 0);
        assert_eq!(state.letters_practice_correct, 0);
        assert_eq!(state.letters_active_days, 0);
        assert_eq!(state.letters_chapters_met, 0);
    }

    #[test]
    fn gate_fail_open_without_letters_deck() {
        // letters_total == 0 (deck not loaded): the gate must never brick.
        let db = Database::open(":memory:", None).unwrap();
        let state = Curriculum::alphabet_gate_state(&db, &[]);
        assert!(state.satisfied);
        assert_eq!(state.method, None);
        assert_eq!(state.letters_total, 0);
    }

    #[test]
    fn gate_grandfathered_by_vocab_history() {
        let db = Database::open(":memory:", None).unwrap();
        // One non-letters memory state (the owner's old vocab data) suffices.
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-001', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                [],
            )
            .unwrap();
        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(state.satisfied);
        assert_eq!(state.method, Some(AlphabetGateMethod::VocabHistory));
        // Letters-only rows must NOT trigger the lane.
        let db2 = Database::open(":memory:", None).unwrap();
        db2.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('letters-001', 30.0, 5.0, '2024-01-01T00:00:00Z', 5, 0)",
                [],
            )
            .unwrap();
        assert!(!Curriculum::alphabet_gate_state(&db2, &letters_deck()).satisfied);
    }

    #[test]
    fn gate_organic_lane() {
        // 43 letters states + 20 practice attempts at 90% across 2 UTC days.
        let db = Database::open(":memory:", None).unwrap();
        seed_organic_lane(&db, &["2026-10-15", "2026-10-16"], 18, 2);
        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(state.satisfied, "organic lane: {:?}", state);
        assert_eq!(state.method, Some(AlphabetGateMethod::Organic));
        assert_eq!(state.letters_introduced, 43);
        assert_eq!(state.letters_practice_total, 20);
        assert_eq!(state.letters_practice_correct, 18);
        assert_eq!(state.letters_active_days, 2);

        // Same accuracy on ONE day only: the spacing hedge fails.
        let one_day = Database::open(":memory:", None).unwrap();
        seed_organic_lane(&one_day, &["2026-10-15"], 18, 2);
        assert!(!Curriculum::alphabet_gate_state(&one_day, &letters_deck()).satisfied);

        // 80% accuracy (16/20) over 2 days: below the 85% bar.
        let sloppy = Database::open(":memory:", None).unwrap();
        seed_organic_lane(&sloppy, &["2026-10-15", "2026-10-16"], 16, 4);
        assert!(!Curriculum::alphabet_gate_state(&sloppy, &letters_deck()).satisfied);

        // Under-20 attempts, even at 100%, is not enough.
        let thin = Database::open(":memory:", None).unwrap();
        seed_organic_lane(&thin, &["2026-10-15", "2026-10-16"], 19, 0);
        assert!(!Curriculum::alphabet_gate_state(&thin, &letters_deck()).satisfied);
    }

    #[test]
    fn gate_latch_persists() {
        let db = Database::open(":memory:", None).unwrap();
        // Some letters activity first, then the check-pass latch.
        seed_organic_lane(&db, &["2026-10-15"], 3, 1);
        Curriculum::record_alphabet_gate(&db, AlphabetGateMethod::CheckPassed).unwrap();
        // Wipe all underlying evidence: the latch still holds.
        db.conn()
            .execute_batch("DELETE FROM memory_states; DELETE FROM review_logs;")
            .unwrap();
        let state = Curriculum::alphabet_gate_state(&db, &letters_deck());
        assert!(state.satisfied);
        assert_eq!(state.method, Some(AlphabetGateMethod::CheckPassed));
        assert_eq!(state.letters_introduced, 0);
        let at: String = db
            .conn()
            .query_row(
                "SELECT value FROM user_settings WHERE key = 'alphabet_gate_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(at.len() >= 20 && at.contains('T'), "ISO-8601 timestamp: {at}");
    }

    #[test]
    fn evaluate_alphabet_check_bounds() {
        assert!(Curriculum::evaluate_alphabet_check(22, 24));
        assert!(Curriculum::evaluate_alphabet_check(24, 24));
        assert!(Curriculum::evaluate_alphabet_check(23, 30));
        assert!(!Curriculum::evaluate_alphabet_check(21, 24));
        // A short check can never pass, however accurate.
        assert!(!Curriculum::evaluate_alphabet_check(23, 23));
    }

    #[test]
    fn alphabet_gate_method_names_match_serde() {
        for (method, name) in [
            (AlphabetGateMethod::CheckPassed, "check_passed"),
            (AlphabetGateMethod::Organic, "organic"),
            (AlphabetGateMethod::DeclaredKnown, "declared_known"),
            (AlphabetGateMethod::VocabHistory, "vocab_history"),
        ] {
            assert_eq!(method.name(), name);
            // The persisted name is exactly the serde snake-case form.
            assert_eq!(serde_json::to_string(&method).unwrap(), format!("\"{name}\""));
            assert_eq!(AlphabetGateMethod::from_name(name), Some(method));
        }
        assert_eq!(AlphabetGateMethod::from_name("nonsense"), None);
    }

    #[test]
    fn letters_exposure_rule_vs_vocab_30d() {
        let db = Database::open(":memory:", None).unwrap();
        // Identical low-stability rows (stability = 2 days, reps = 2) on
        // letters ch.1 AND athenaze ch.1.
        for i in 1..=7 {
            db.conn()
                .execute(
                    "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                     VALUES (?1, 2.0, 5.0, '2024-01-01T00:00:00Z', 2, 0)",
                    [format!("letters-{i:03}")],
                )
                .unwrap();
        }
        for i in 1..=5 {
            db.conn()
                .execute(
                    "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                     VALUES (?1, 2.0, 5.0, '2024-01-01T00:00:00Z', 2, 0)",
                    [format!("ath1-{i:03}")],
                )
                .unwrap();
        }
        let mut letters: Vec<Card> = (1..=7)
            .map(|i| letters_card(&format!("letters-{i:03}"), 1))
            .collect();
        letters.push(letters_card("letters-008", 2));
        let vocab: Vec<Card> = (1..=10)
            .map(|i| sample_card(&format!("ath1-{i:03}"), if i <= 5 { 1 } else { 2 }, "athenaze-i"))
            .collect();

        // Letters: reps >= 2 counts — all 7 ch.1 cards "met", requirement
        // (6 of 7) satisfied, hint clears, and the state reports 1 met chapter.
        let chapters = Curriculum::get_chapters(&db, "letters", &letters);
        assert_eq!(chapters[0].mastered_count, 7);
        assert!(!chapters[1].unlocked);
        assert_eq!(chapters[1].unlock_hint, "");
        let mut all_cards = letters.clone();
        all_cards.extend(vocab.iter().cloned());
        let state = Curriculum::alphabet_gate_state(&db, &all_cards);
        assert_eq!(state.letters_chapters_met, 1);

        // Athenaze: the 30-day stability rule still applies, so the identical
        // rows count for nothing. (The vocab rows also grandfather the gate,
        // so the normal prev-chapter hint shows rather than the gate hint.)
        let chapters = Curriculum::get_chapters(&db, "athenaze", &vocab);
        assert_eq!(chapters[0].mastered_count, 0);
        assert_eq!(
            chapters[1].unlock_hint,
            "Master 80% of Chapter 1 to unlock (0/4)"
        );
    }

    #[test]
    fn gate_locks_vocab_chapters_with_hint() {
        let db = Database::open(":memory:", None).unwrap();
        let vocab: Vec<Card> = (1..=10)
            .map(|i| sample_card(&format!("ath1-{i:03}"), if i <= 5 { 1 } else { 2 }, "athenaze-i"))
            .collect();
        // Letters deck loaded (so the gate is live) but no progress at all.
        let mut cards = letters_deck();
        cards.extend(vocab.iter().cloned());

        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        // While the gate is unmet, even the default-unlocked ch.1 reports
        // locked and carries the gate hint; later chapters keep their
        // existing prev-chapter hints.
        assert!(chapters.iter().all(|c| !c.unlocked));
        assert_eq!(
            chapters[0].unlock_hint,
            "Finish 'The Alphabet & Pronunciation' — or pass the Alphabet Check."
        );
        assert_eq!(
            chapters[1].unlock_hint,
            "Master 80% of Chapter 1 to unlock (0/4)"
        );
        // After passing the check, ch.1 is back to default-unlocked.
        Curriculum::record_alphabet_gate(&db, AlphabetGateMethod::CheckPassed).unwrap();
        let chapters = Curriculum::get_chapters(&db, "athenaze", &cards);
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].unlock_hint, "");
        assert!(!chapters[1].unlocked);
        assert_eq!(
            chapters[1].unlock_hint,
            "Master 80% of Chapter 1 to unlock (0/4)"
        );
        // The letters track itself is never gate-locked.
        let chapters = Curriculum::get_chapters(&db, "letters", &cards);
        assert!(chapters[0].unlocked);
    }
}
