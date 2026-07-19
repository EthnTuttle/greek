use anyhow::Result;
use rusqlite::Connection;

pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open or create the main app database. Creates tables if they don't exist.
    /// `db_path` is the path to the app's SQLite database.
    /// `morphology_db_path` is the path to the morphology.sqlite (attached as 'morph').
    pub fn open(db_path: &str, morphology_db_path: Option<&str>) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        // Enable WAL mode for better concurrency
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        // Create app tables
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS memory_states(
                card_id TEXT PRIMARY KEY,
                stability REAL NOT NULL,
                difficulty REAL NOT NULL,
                last_review TEXT,
                due_date TEXT NOT NULL,
                reps INTEGER NOT NULL DEFAULT 0,
                lapses INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS review_logs(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                card_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                rating INTEGER NOT NULL,
                elapsed_days REAL NOT NULL,
                scheduled_days REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS curriculum_progress(
                textbook TEXT NOT NULL,
                chapter INTEGER NOT NULL,
                unlocked INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (textbook, chapter)
            );

            CREATE TABLE IF NOT EXISTS user_settings(
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS known_words(
                word TEXT PRIMARY KEY,
                known INTEGER NOT NULL DEFAULT 1,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS lookup_cards(
                card_id TEXT PRIMARY KEY,
                front TEXT NOT NULL,
                back TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_memory_states_due_date
                ON memory_states(due_date);
            ",
        )?;

        // Attach morphology database if provided
        if let Some(morph_path) = morphology_db_path {
            conn.execute_batch(&format!(
                "ATTACH DATABASE '{}' AS morph;",
                morph_path.replace('\'', "''")
            ))?;
        }

        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = Database::open(":memory:", None).unwrap();
        // Verify tables exist
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_states'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_insert_and_read_memory_state() {
        let db = Database::open(":memory:", None).unwrap();
        db.conn()
            .execute(
                "INSERT INTO memory_states (card_id, stability, difficulty, due_date, reps, lapses)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params!["card-001", 5.0, 5.0, "2024-01-01T00:00:00Z", 0, 0],
            )
            .unwrap();

        let stability: f64 = db
            .conn()
            .query_row(
                "SELECT stability FROM memory_states WHERE card_id = ?1",
                ["card-001"],
                |row| row.get(0),
            )
            .unwrap();
        assert!((stability - 5.0).abs() < f64::EPSILON);
    }
}
