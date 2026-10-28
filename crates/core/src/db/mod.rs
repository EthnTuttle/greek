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
/// - 3: sync-ready merge layer — `review_logs.uuid/device_id/hlc` columns
///   (existing rows backfilled with random v4 uuids, the local device id, and
///   an HLC derived from the row timestamp), plus `settings_sync` and
///   `sync_devices` tables and a unique index on `review_logs.uuid`
/// - 4: `secrets` table — never synced, never in backup exports. The
///   migration MOVES any `nostr_nsec` rows out of `user_settings` /
///   `settings_sync` into `secrets` and deletes the synced/plaintext copies
///   (DESIGN-progress-sharing.md, Appendix R "Secrets").
/// - 5: C2 contents cleanup — delete legacy zero-state `memory_states` rows
///   written by the old `add_card_from_lookup` (stability/difficulty 0.0,
///   reps=0, last_review NULL, no review history). Such a row masqueraded as
///   a learned FSRS state: bogus fixed previews and incremental-vs-replay
///   divergence. An unreviewed lookup card must have NO row at all.
/// - 6: S6 `sync_applied_batches(batch_uuid, applied_at)` — nostr pull
///   admission dedups per wire batch UUID (the chunk "b" tag) instead of a
///   single global HLC cursor, so a previously-incomplete or lagging-clock
///   batch whose HLCs sit below the pull high-water mark can still apply
///   when it later completes (see `sync::mark_batch_applied`).
const SCHEMA_VERSION: i64 = 6;

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

    /// Run `f` inside a single IMMEDIATE transaction: commit on success,
    /// roll back and propagate the error on failure. [`Database`] only
    /// exposes `&Connection` (no `&mut`, so rusqlite's `transaction()` is
    /// unavailable), hence explicit BEGIN/COMMIT — `f` receives this same
    /// connection handle, and any other statement issued on it while the
    /// transaction is open (e.g. the HLC advance in `sync::hlc_now`) joins
    /// the transaction and rolls back together.
    pub fn with_tx<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        run_in_tx(&self.conn, f)
    }
}

/// Run `f` inside a single IMMEDIATE transaction on `conn`: commit on
/// success, roll back and propagate the error on failure. Nesting-aware:
/// when `conn` is already inside a transaction (an enclosing `with_tx`
/// composed this call, e.g. the backup-import wrapper enveloping
/// `apply_review_events` + `apply_settings`), `f` just runs against the
/// open transaction — the OUTER envelope keeps sole commit/rollback
/// authority, so a late error anywhere still rolls back the whole unit.
fn run_in_tx<T>(conn: &Connection, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    if !conn.is_autocommit() {
        return f(conn);
    }
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    match f(conn) {
        Ok(result) => {
            conn.execute_batch("COMMIT;")?;
            Ok(result)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
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
        //
        // ATOMICITY (S5): ALL pending steps PLUS the `user_version` stamp
        // run inside ONE IMMEDIATE transaction — a crash or error mid-way
        // rolls everything back instead of leaving half-applied ALTERs
        // behind an unbumped version stamp (which would make the next open
        // retry already-committed column additions and wedge on
        // "duplicate column name"). `PRAGMA user_version = N` is
        // transactional in SQLite and rides the same envelope.
        run_in_tx(conn, |conn| {
        if version < 2 {
            // The baseline CREATE TABLE above deliberately omits `source` so
            // that fresh (v0) and legacy (v1) databases converge through this
            // single ALTER. Existing rows read 'flashcard' via the DEFAULT.
            conn.execute_batch(
                "ALTER TABLE review_logs ADD COLUMN source TEXT NOT NULL DEFAULT 'flashcard';",
            )?;
        }

        if version < 3 {
            // Sync merge layer (see crate::sync): every review row becomes a
            // globally-addressable CRDT event (uuid + device_id + HLC), and
            // settings/devices get their own sync tables. Same convergence
            // pattern as v2: fresh databases reach this shape through the
            // ALTERs, so the CREATE TABLE set above stays at the baseline.
            conn.execute_batch(
                "ALTER TABLE review_logs ADD COLUMN uuid TEXT;
                 ALTER TABLE review_logs ADD COLUMN device_id TEXT;
                 ALTER TABLE review_logs ADD COLUMN hlc TEXT;
                 CREATE TABLE IF NOT EXISTS settings_sync(
                     key TEXT PRIMARY KEY,
                     value TEXT NOT NULL,
                     hlc TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS sync_devices(
                     device_id TEXT PRIMARY KEY,
                     label TEXT NOT NULL DEFAULT '',
                     paired_at TEXT NOT NULL,
                     last_sync_hlc TEXT
                 );",
            )?;

            // Bootstrap this device's id BEFORE stamping rows with it.
            conn.execute(
                "INSERT INTO user_settings (key, value) VALUES ('device_id', ?1)
                 ON CONFLICT(key) DO NOTHING",
                [uuid::Uuid::new_v4().to_string()],
            )?;
            let dev_id: String = conn.query_row(
                "SELECT value FROM user_settings WHERE key = 'device_id'",
                [],
                |row| row.get(0),
            )?;
            let node = crate::sync::hlc_node_of(&dev_id);

            // Backfill pre-sync rows: uuid = random v4, device = this device,
            // hlc derived from the row's own timestamp (counter 0).
            let legacy: Vec<(i64, String)> = {
                let mut stmt =
                    conn.prepare("SELECT id, timestamp FROM review_logs WHERE uuid IS NULL")?;
                let rows = stmt
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                rows
            };
            for (row_id, ts) in legacy {
                let ms = crate::srs::iso_to_secs(&ts).map(|s| (s * 1000) as i64).unwrap_or(0);
                let hlc = format!("{ms:013}-{0:04}-{node}", 0u16);
                conn.execute(
                    "UPDATE review_logs SET uuid = ?1, device_id = ?2, hlc = ?3 WHERE id = ?4",
                    rusqlite::params![uuid::Uuid::new_v4().to_string(), dev_id, hlc, row_id],
                )?;
            }

            conn.execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_review_logs_uuid ON review_logs(uuid);",
            )?;
        }

        if version < 4 {
            // Secrets vault: values that must never ride settings sync or
            // backup exports. Move any pre-v4 `nostr_nsec` copies out of
            // BOTH settings tables into it. INSERT OR IGNORE keeps an
            // existing secrets row (user_settings wins over settings_sync
            // on first run), so the step is safely re-runnable.
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS secrets(
                     key TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );
                 INSERT OR IGNORE INTO secrets (key, value)
                     SELECT key, value FROM user_settings WHERE key = 'nostr_nsec';
                 INSERT OR IGNORE INTO secrets (key, value)
                     SELECT key, value FROM settings_sync WHERE key = 'nostr_nsec';
                 DELETE FROM user_settings WHERE key = 'nostr_nsec';
                 DELETE FROM settings_sync WHERE key = 'nostr_nsec';",
            )?;
        }

        if version < 5 {
            // C2: purge legacy zero-state lookup rows. The signature is a
            // never-reviewed placeholder: reps=0, last_review NULL, AND no
            // review_logs row for the card (the log guard makes the step
            // safe even against a torn first-review write). The `lookup-%`
            // prefix guard means genuine deck cards can never match: the
            // only writer of reps=0/NULL rows was the old
            // add_card_from_lookup. `lookup-%` contains no LIKE wildcards
            // itself, so no escape clause is needed.
            conn.execute_batch(
                "DELETE FROM memory_states
                 WHERE card_id LIKE 'lookup-%'
                   AND reps = 0
                   AND last_review IS NULL
                   AND card_id NOT IN (SELECT DISTINCT card_id FROM review_logs);",
            )?;
        }

        if version < 6 {
            // S6 pull admission ledger: one row per APPLIED wire batch UUID.
            // Rows are inserted by `sync::mark_batch_applied` in the SAME
            // transaction as the batch's application, so a crash can never
            // record an unapplied batch (it is simply retried next pull).
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS sync_applied_batches(
                     batch_uuid TEXT PRIMARY KEY,
                     applied_at TEXT NOT NULL
                 );",
            )?;
        }

        // Stamp last, INSIDE the envelope: the version only ever advances
        // together with the schema changes it describes.
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        Ok(())
        })
    } else {
        Ok(())
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
    fn test_migrate_v5_to_v6_creates_sync_applied_batches() {
        // Build a v5-shaped database by hand: pre-existing data rows,
        // user_version stamped at 5, no sync_applied_batches table.
        let dir = std::env::temp_dir().join(format!("greek-v5-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v5.db");
        let path_str = path.to_string_lossy().into_owned();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE user_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE review_logs(
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     card_id TEXT NOT NULL,
                     timestamp TEXT NOT NULL,
                     rating INTEGER NOT NULL,
                     elapsed_days REAL NOT NULL,
                     scheduled_days REAL NOT NULL,
                     source TEXT NOT NULL DEFAULT 'flashcard',
                     uuid TEXT,
                     device_id TEXT,
                     hlc TEXT
                 );
                 INSERT INTO user_settings (key, value) VALUES ('device_id', 'dev-v5');
                 INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days, uuid, device_id, hlc)
                     VALUES ('old-card', '2024-01-01T00:00:00Z', 3, 0.0, 1.0,
                             'row-uuid-1', 'dev-v5', '0000000000001-0000-abcd1234');
                 PRAGMA user_version = 5;",
            )
            .unwrap();
        }

        // Opening upgrades to v6: the ledger table appears, data intact.
        let db = Database::open(&path_str, None).unwrap();
        let version: i64 = db
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let n: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sync_applied_batches'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "sync_applied_batches must be created by the v6 step");
        let device_id: String = db
            .conn()
            .query_row("SELECT value FROM user_settings WHERE key = 'device_id'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(device_id, "dev-v5", "v5 data must survive the migration");
        let rating: i64 = db
            .conn()
            .query_row("SELECT rating FROM review_logs WHERE uuid = 'row-uuid-1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rating, 3);
        // The ledger starts EMPTY: pre-v6 databases may have pushed batches
        // already; those are uuid-deduped by apply_review_events on their
        // first post-upgrade (re)apply and then marked.
        let ledger: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM sync_applied_batches", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ledger, 0);
        drop(db);

        // Idempotent reopen: version stays, no re-run, table still there.
        let db2 = Database::open(&path_str, None).unwrap();
        let v2: i64 = db2
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v2, SCHEMA_VERSION);
        drop(db2);
        let _ = std::fs::remove_dir_all(&dir);
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
        assert_eq!(
            version, SCHEMA_VERSION,
            "user_version must advance to the current schema"
        );
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
    fn test_migrate_v3_to_v4_moves_nostr_nsec_into_secrets() {
        // Build a v3-shaped database by hand: user_settings + settings_sync
        // holding a nostr_nsec (and an ordinary setting), user_version = 3.
        let dir = std::env::temp_dir().join(format!("greek-v3-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v3.db");
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
                     scheduled_days REAL NOT NULL,
                     source TEXT NOT NULL DEFAULT 'flashcard',
                     uuid TEXT,
                     device_id TEXT,
                     hlc TEXT
                 );
                 CREATE TABLE user_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE settings_sync(key TEXT PRIMARY KEY, value TEXT NOT NULL, hlc TEXT NOT NULL);
                 CREATE TABLE sync_devices(
                     device_id TEXT PRIMARY KEY,
                     label TEXT NOT NULL DEFAULT '',
                     paired_at TEXT NOT NULL,
                     last_sync_hlc TEXT
                 );
                 CREATE UNIQUE INDEX idx_review_logs_uuid ON review_logs(uuid);
                 INSERT INTO user_settings (key, value)
                     VALUES ('nostr_nsec', 'deadbeefdeadbeef'),
                            ('theme', 'dark');
                 INSERT INTO settings_sync (key, value, hlc)
                     VALUES ('nostr_nsec', 'deadbeefdeadbeef', '0000000000001-0000-aabbccdd'),
                            ('theme', 'dark', '0000000000001-0000-aabbccdd');
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        }

        // Opening must upgrade to v4: secret moved, settings purged, the
        // ordinary setting untouched.
        let db = Database::open(&path_str, None).unwrap();
        let version: i64 = db
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        let secret: String = db
            .conn()
            .query_row("SELECT value FROM secrets WHERE key = 'nostr_nsec'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(secret, "deadbeefdeadbeef");

        for table in ["user_settings", "settings_sync"] {
            let n: i64 = db
                .conn()
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE key = 'nostr_nsec'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{table} must not retain the secret");
            let theme: String = db
                .conn()
                .query_row(
                    &format!("SELECT value FROM {table} WHERE key = 'theme'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(theme, "dark", "{table} must keep non-secret rows");
        }

        // Fresh databases get the secrets table too.
        let fresh = Database::open(":memory:", None).unwrap();
        let n: i64 = fresh
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='secrets'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // S5: a migration interrupted mid-flight must roll back EVERYTHING —
    // after the failed open, the version stamp is unchanged and none of the
    // midway ALTERs exist, so a later open (obstruction removed) retries the
    // whole migration cleanly instead of wedging on duplicate columns.
    // Simulated via a tripwire trigger that ABORTs the v3 step's first
    // user_settings INSERT.
    #[test]
    fn test_interrupted_migration_rolls_back_and_retries_cleanly() {
        let dir = std::env::temp_dir().join(format!("greek-tx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("interrupted.db");
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
                     scheduled_days REAL NOT NULL,
                     source TEXT NOT NULL DEFAULT 'flashcard'
                 );
                 CREATE TABLE user_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO review_logs (card_id, timestamp, rating, elapsed_days, scheduled_days)
                     VALUES ('c1', '2024-01-01T00:00:00Z', 3, 0.0, 1.0);
                 CREATE TRIGGER tripwire BEFORE INSERT ON user_settings
                     BEGIN SELECT RAISE(ABORT, 'simulated interruption'); END;
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        }

        // First open: migration hits the tripwire mid-v3-step and fails.
        assert!(
            Database::open(&path_str, None).is_err(),
            "the tripwire must abort the migration"
        );

        // Nothing half-applied survives: version still 2, no `uuid` column.
        {
            let conn = Connection::open(&path).unwrap();
            let v: i64 = conn
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(v, 2, "an aborted migration must not advance the stamp");
            let cols: Vec<String> = {
                let mut stmt = conn.prepare("PRAGMA table_info(review_logs)").unwrap();
                stmt.query_map([], |row| row.get(1))
                    .unwrap()
                    .collect::<std::result::Result<Vec<String>, _>>()
                    .unwrap()
            };
            assert!(
                !cols.iter().any(|c| c == "uuid"),
                "interrupted ALTERs must roll back, found columns: {cols:?}"
            );
            // Remove the obstruction; the retry below must see a pristine v2.
            conn.execute_batch("DROP TRIGGER tripwire;").unwrap();
        }

        // Retry: the FULL migration runs from scratch and converges to the
        // current schema — no "duplicate column name" wedge.
        let db = Database::open(&path_str, None).unwrap();
        let v: i64 = db
            .conn()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let backfilled: Option<String> = db
            .conn()
            .query_row("SELECT uuid FROM review_logs WHERE card_id = 'c1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(backfilled.is_some(), "retry must run the v3 backfill");

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
