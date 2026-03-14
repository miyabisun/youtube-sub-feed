use rusqlite::Connection;

pub fn open(path: &str) -> Connection {
    tracing::info!("Database: {}", path);
    let conn = Connection::open(path).expect("Failed to open database");

    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA cache_size = -64000;
         PRAGMA temp_store = MEMORY;
         PRAGMA foreign_keys = ON;",
    )
    .expect("Failed to set PRAGMA");

    create_tables(&conn);

    conn
}

pub fn open_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("Failed to open in-memory database");

    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("Failed to set PRAGMA");

    create_tables(&conn);

    conn
}

fn create_tables(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS channels (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            thumbnail_url TEXT,
            upload_playlist_id TEXT,
            show_livestreams INTEGER NOT NULL DEFAULT 0,
            last_fetched_at TEXT,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS videos (
            id TEXT PRIMARY KEY,
            channel_id TEXT NOT NULL,
            title TEXT NOT NULL,
            thumbnail_url TEXT,
            published_at TEXT,
            duration TEXT,
            is_short INTEGER NOT NULL DEFAULT 0,
            is_livestream INTEGER NOT NULL DEFAULT 0,
            livestream_ended_at TEXT,
            is_hidden INTEGER NOT NULL DEFAULT 0,
            fetched_at TEXT,
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS channel_groups (
            channel_id TEXT NOT NULL,
            group_id INTEGER NOT NULL,
            PRIMARY KEY (channel_id, group_id),
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
            FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS auth (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            google_id TEXT NOT NULL,
            email TEXT NOT NULL,
            access_token TEXT,
            refresh_token TEXT,
            token_expires_at TEXT,
            updated_at TEXT
        );

        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            auth_id INTEGER NOT NULL,
            expires_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (auth_id) REFERENCES auth(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_videos_published ON videos (published_at DESC);
        CREATE INDEX IF NOT EXISTS idx_videos_channel ON videos (channel_id);
        CREATE INDEX IF NOT EXISTS idx_videos_hidden ON videos (is_hidden, published_at DESC);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_auth_google_id ON auth(google_id);",
    )
    .expect("Failed to create tables");
}

#[cfg(test)]
mod tests {
    // Database Schema Spec
    //
    // SQLite with 6 tables. Raw SQL without ORM.
    // Tables are auto-created on startup via `CREATE TABLE IF NOT EXISTS` (no migration needed).

    use super::*;

    #[test]
    fn test_open_memory() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_foreign_keys_cascade() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('v1', 'UC1', 'Video', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        // Delete channel should cascade to videos
        conn.execute("DELETE FROM channels WHERE id = 'UC1'", [])
            .unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_all_tables_exist() {
        let conn = open_memory();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        let expected = ["auth", "channel_groups", "channels", "groups", "sessions", "videos"];
        for name in &expected {
            assert!(tables.contains(&name.to_string()), "Table '{}' not found", name);
        }
    }

    #[test]
    fn test_indexes_exist() {
        let conn = open_memory();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_%' ORDER BY name")
            .unwrap();
        let indexes: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        let expected = [
            "idx_auth_google_id",
            "idx_videos_channel",
            "idx_videos_hidden",
            "idx_videos_published",
        ];
        for name in &expected {
            assert!(indexes.contains(&name.to_string()), "Index '{}' not found", name);
        }
    }

    #[test]
    fn test_google_id_unique() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO auth (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO auth (google_id, email) VALUES ('g1', 'b@example.com')",
            [],
        );
        assert!(result.is_err(), "Duplicate google_id should fail");
    }

    #[test]
    fn test_idempotent_ddl() {
        let _conn1 = open_memory();
        let _conn2 = open_memory();
        // Both calls succeed without error thanks to CREATE TABLE IF NOT EXISTS
    }

    #[test]
    fn test_video_default_values() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC1', 'Video')",
            [],
        )
        .unwrap();

        let (is_short, is_livestream, is_hidden): (i64, i64, i64) = conn
            .query_row(
                "SELECT is_short, is_livestream, is_hidden FROM videos WHERE id = 'v1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(is_short, 0);
        assert_eq!(is_livestream, 0);
        assert_eq!(is_hidden, 0);
    }

    #[test]
    fn test_channel_default_values() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let show_livestreams: i64 = conn
            .query_row(
                "SELECT show_livestreams FROM channels WHERE id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(show_livestreams, 0);
    }

    #[test]
    fn test_channel_groups_cascade_on_group_delete() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO groups (name, created_at) VALUES ('Group1', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let group_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', ?1)",
            [group_id],
        )
        .unwrap();

        // Delete group should cascade to channel_groups
        conn.execute("DELETE FROM groups WHERE id = ?1", [group_id])
            .unwrap();

        let cg_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channel_groups", [], |row| row.get(0))
            .unwrap();
        assert_eq!(cg_count, 0, "channel_groups row should be deleted");

        let ch_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ch_count, 1, "Channel should still exist");
    }

    #[test]
    fn test_sessions_cascade_on_auth_delete() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO auth (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        let auth_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES ('s1', ?1, '2025-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            [auth_id],
        )
        .unwrap();

        // Delete auth should cascade to sessions
        conn.execute("DELETE FROM auth WHERE id = ?1", [auth_id])
            .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "Session should be deleted when auth is deleted");
    }

    #[test]
    fn test_last_fetched_at_is_nullable() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC2', 'ch', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let val: Option<String> = conn
            .query_row(
                "SELECT last_fetched_at FROM channels WHERE id = 'UC2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(val.is_none(), "last_fetched_at should be NULL before first fetch");
    }

    #[test]
    fn test_duplicate_id_fails_primary_key() {
        let conn = open_memory();
        let insert = "INSERT INTO channels (id, title, created_at) VALUES ('UC3', 'ch', '2025-01-01T00:00:00Z')";
        conn.execute(insert, []).unwrap();
        assert!(conn.execute(insert, []).is_err(), "duplicate PK insert should fail");
    }

    #[test]
    fn test_upsert_updates_utf8_title_and_thumbnail() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'ch', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('vid1', 'UC1', 'video', '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO videos (id, channel_id, title, thumbnail_url, fetched_at)
             VALUES ('vid1', 'UC1', '新しいタイトル', 'new_url', '2025-06-01T00:00:00Z')
             ON CONFLICT(id) DO UPDATE SET
               title = excluded.title,
               thumbnail_url = excluded.thumbnail_url
             WHERE title != excluded.title OR thumbnail_url != excluded.thumbnail_url",
            [],
        )
        .unwrap();

        let title: String = conn
            .query_row("SELECT title FROM videos WHERE id = 'vid1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(title, "新しいタイトル", "UPSERT should update title to follow author edits");
    }

    // M2M test (channel belongs to multiple groups) is in routes/groups.rs

    #[test]
    fn test_duplicate_composite_pk_fails() {
        let conn = open_memory();
        conn.execute_batch(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'ch1', '2025-01-01T00:00:00Z');
             INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 1, '2025-01-01T00:00:00Z');",
        )
        .unwrap();

        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)", []).unwrap();
        let result = conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)", []);
        assert!(result.is_err(), "composite PK (channel_id, group_id) prevents duplicates");
    }

    #[test]
    fn test_insert_and_select_utf8_channel() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UCxxxxxxxx', 'テストチャンネル', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let title: String = conn
            .query_row(
                "SELECT title FROM channels WHERE id = 'UCxxxxxxxx'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "テストチャンネル");
    }

    #[test]
    fn test_insert_video_with_nonexistent_channel_fails_fk() {
        let conn = open_memory();
        let result = conn.execute(
            "INSERT INTO videos (id, channel_id, title) VALUES ('vid1', 'UC_nonexistent', 'v')",
            [],
        );
        assert!(result.is_err(), "FK constraint rejects nonexistent channel_id");
    }
}
