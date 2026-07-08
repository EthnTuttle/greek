use crate::db::Database;
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
        ]
    }

    /// Get chapters for a given textbook with progress info
    pub fn get_chapters(db: &Database, textbook: &str) -> Vec<ChapterInfo> {
        let info = Self::get_textbooks();
        let total_chapters = info
            .iter()
            .find(|t| t.textbook == textbook)
            .map(|t| t.total_chapters)
            .unwrap_or(0);

        let mut chapters = Vec::new();

        for chapter in 1..=total_chapters {
            // Check if chapter is unlocked
            let unlocked: bool = db
                .conn()
                .query_row(
                    "SELECT unlocked FROM curriculum_progress WHERE textbook = ?1 AND chapter = ?2",
                    rusqlite::params![textbook, chapter],
                    |row| row.get::<_, i32>(0).map(|v| v != 0),
                )
                .unwrap_or(chapter == 1); // Chapter 1 is always unlocked by default

            // Count mastered cards (stability > 30 days) for this chapter
            // In a real app, cards would have a chapter field in the DB
            // For now, mastered_count is 0 since we don't have a cards table yet
            let mastered_count = 0;
            let card_count = 0;

            let title = chapter_title(textbook, chapter);

            chapters.push(ChapterInfo {
                chapter,
                title,
                card_count,
                mastered_count,
                unlocked,
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
        _ => format!("Chapter {}", chapter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_textbooks() {
        let textbooks = Curriculum::get_textbooks();
        assert_eq!(textbooks.len(), 2);
        assert_eq!(textbooks[0].textbook, "athenaze");
        assert_eq!(textbooks[1].textbook, "jact");
    }

    #[test]
    fn test_get_chapters() {
        let db = Database::open(":memory:", None).unwrap();
        let chapters = Curriculum::get_chapters(&db, "athenaze");
        assert_eq!(chapters.len(), 31);
        // Chapter 1 should be unlocked by default
        assert!(chapters[0].unlocked);
        assert_eq!(chapters[0].title, "Ο ΔΙΚΑΙΟΠΟΛΙΣ (Dicaeopolis)");
    }

    #[test]
    fn test_unlock_chapter() {
        let db = Database::open(":memory:", None).unwrap();

        // Chapter 2 should not be unlocked initially
        let chapters = Curriculum::get_chapters(&db, "athenaze");
        assert!(!chapters[1].unlocked);

        // Unlock chapter 2
        Curriculum::unlock_chapter(&db, "athenaze", 2).unwrap();

        // Now it should be unlocked
        let chapters = Curriculum::get_chapters(&db, "athenaze");
        assert!(chapters[1].unlocked);
    }
}
