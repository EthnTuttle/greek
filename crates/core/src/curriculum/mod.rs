use crate::db::Database;
use crate::srs::Card;
use serde::{Deserialize, Serialize};

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
    /// and `mastered_count` are computed from it (mastered = has a memory_states
    /// row with reps >= 1). Chapters with no cards are marked `available == false`
    /// so the UI can render them as "Coming Soon".
    pub fn get_chapters(db: &Database, textbook: &str, cards: &[Card]) -> Vec<ChapterInfo> {
        let info = Self::get_textbooks();
        let total_chapters = info
            .iter()
            .find(|t| t.textbook == textbook)
            .map(|t| t.total_chapters)
            .unwrap_or(0);

        // Set of card ids that have been reviewed at least once (reps >= 1).
        let mastered_ids: std::collections::HashSet<String> = db
            .conn()
            .prepare("SELECT card_id FROM memory_states WHERE reps >= 1")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))
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

            let title = chapter_title(textbook, chapter);

            chapters.push(ChapterInfo {
                chapter,
                title,
                card_count,
                mastered_count,
                unlocked,
                available,
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
        assert_eq!(textbooks.len(), 3);
        assert_eq!(textbooks[0].textbook, "athenaze");
        assert_eq!(textbooks[1].textbook, "jact");
        assert_eq!(textbooks[2].textbook, "dcc");
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
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES ('ath1-001', 5.0, 5.0, '2024-01-01T00:00:00Z', 3, 0)",
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
}
