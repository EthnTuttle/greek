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

    /// Get chapters for a given textbook with progress info.
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

        // Set of card ids that count as mastered (stability >= 30 days).
        let mastered_ids: std::collections::HashSet<String> = db
            .conn()
            .prepare("SELECT card_id FROM memory_states WHERE stability >= ?1")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([MASTERY_STABILITY_DAYS as f64], |row| {
                    row.get::<_, String>(0)
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

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
}
