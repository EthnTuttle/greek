use anyhow::Result;
use rusqlite::Connection;

pub struct Database {
    conn: Connection,
}

/// Current schema version, persisted in SQLite's `user_version` pragma.
///
/// Version history:
/// - 0: no version marker (databases created before migrations existed)
/// - 1: baseline schema — the CREATE TABLE IF NOT EXISTS set in [`Database::open`]
/// - 2: `review_logs.source` column (NOT NULL DEFAULT 'flashcard') —
///   distinguishes self-rated flashcard reviews from practice-question results
const SCHEMA_VERSION: i64 = 2;

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

        // Stamp (or advance) the schema version. No-op for existing databases.
        migrate(&conn)?;

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

/// Apply pending schema migrations and record the current schema version in
/// `PRAGMA user_version`. Idempotent: opening an up-to-date database is a
/// no-op.
fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version < SCHEMA_VERSION {
        // Add ALTER TABLE migrations here as version bumps 1 → 2, 2 → 3, …
        // (see the v2 step below for the pattern), then bump SCHEMA_VERSION
        // to the highest applied step. The stamp below records it. Keep the
        // steps idempotent where possible.

        if version < 2 {
            // The baseline CREATE TABLE above deliberately omits `source` so
            // that fresh (v0) and legacy (v1) databases converge through this
            // single ALTER. Existing rows read 'flashcard' via the DEFAULT.
            conn.execute_batch(
                "ALTER TABLE review_logs ADD COLUMN source TEXT NOT NULL DEFAULT 'flashcard';",
            )?;
        }

        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
    }

    Ok(())
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
    fn test_user_version_migration_is_idempotent() {
        // A shared-cache in-memory DB persists across connections while at
        // least one stays open, so the second `open` sees the version stamped
        // by the first and must be a no-op success.
        let uri = "file:greek_migration_test?mode=memory&cache=shared";

        let db1 = Database::open(uri, None).unwrap();
        let v1: i64 = db1
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v1, SCHEMA_VERSION);

        let db2 = Database::open(uri, None).unwrap();
        let v2: i64 = db2
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v2, SCHEMA_VERSION);
    }

    #[test]
    fn test_migrate_v1_to_v2_adds_source_and_preserves_rows() {
        // Build a v1-shaped database by hand: review_logs WITHOUT the source
        // column, one pre-existing row, user_version stamped at 1.
        let dir = std::env::temp_dir().join(format!("greek-v1-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v1.db");
        let path_str = path.to_string_lossy().into_owned();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE review_logs(
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     card_id TEXT NOT NULL,
                     timestamp TEXT NOT NULL,
                     rating INTEGER NOT NULL,
                     elapsed_days REAL NOT NULL,
                     scheduled_days REAL NOT NULL
                 );
                 INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                     VALUES ('old-card', '2024-01-01T00:00:00Z', 3, 0.0, 1.0);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }

        // Opening must upgrade to v2 and leave legacy rows reading 'flashcard'.
        let db = Database::open(&path_str, None).unwrap();
        let version: i64 = db
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2, "user_version must advance to 2");
        let (rating, source): (i32, String) = db
            .conn()
            .query_row(
                "SELECT rating, source FROM review_logs WHERE card_id = 'old-card'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(rating, 3, "legacy row must survive the ALTER untouched");
        assert_eq!(source, "flashcard", "legacy rows read 'flashcard' via DEFAULT");

        // Fresh databases get the column too (exercise the v0 -> v2 path).
        let fresh = Database::open(":memory:", None).unwrap();
        let source: String = fresh
            .conn()
            .query_row(
                "INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                 VALUES ('c', '2024-01-01T00:00:00Z', 3, 0.0, 0.0)
                 RETURNING source",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source, "flashcard");

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
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
