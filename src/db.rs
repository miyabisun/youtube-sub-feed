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

    migrate(&conn);
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
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            google_id TEXT NOT NULL,
            email TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'member',
            rss_token TEXT,
            access_token TEXT,
            refresh_token TEXT,
            token_expires_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT
        );

        CREATE TABLE IF NOT EXISTS channels (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            thumbnail_url TEXT,
            upload_playlist_id TEXT,
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
            fetched_at TEXT,
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS user_channels (
            user_id INTEGER NOT NULL,
            channel_id TEXT NOT NULL,
            is_favorite INTEGER NOT NULL DEFAULT 0,
            show_livestreams INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (user_id, channel_id),
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS user_videos (
            user_id INTEGER NOT NULL,
            video_id TEXT NOT NULL,
            is_hidden INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (user_id, video_id),
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
            FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS channel_groups (
            channel_id TEXT NOT NULL,
            group_id INTEGER NOT NULL,
            PRIMARY KEY (channel_id, group_id),
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
            FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            expires_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS channel_subscriptions (
            channel_id TEXT PRIMARY KEY,
            hub_secret TEXT NOT NULL,
            lease_seconds INTEGER NOT NULL DEFAULT 0,
            subscribed_at TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            verification_status TEXT NOT NULL DEFAULT 'pending',
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_users_google_id ON users(google_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_users_rss_token ON users(rss_token);
        CREATE INDEX IF NOT EXISTS idx_videos_published ON videos (published_at DESC);
        CREATE INDEX IF NOT EXISTS idx_videos_channel ON videos (channel_id);
        CREATE INDEX IF NOT EXISTS idx_user_channels_user ON user_channels(user_id);
        CREATE INDEX IF NOT EXISTS idx_user_channels_favorite ON user_channels(user_id, is_favorite);
        CREATE INDEX IF NOT EXISTS idx_user_videos_user ON user_videos(user_id);
        CREATE INDEX IF NOT EXISTS idx_user_videos_hidden ON user_videos(user_id, is_hidden);
        CREATE INDEX IF NOT EXISTS idx_groups_user ON groups(user_id);
        CREATE INDEX IF NOT EXISTS idx_channel_subscriptions_expires ON channel_subscriptions(expires_at);",
    )
    .expect("Failed to create tables");
}

fn migrate(conn: &Connection) {
    let has_auth_table = conn
        .prepare("SELECT 1 FROM auth LIMIT 0")
        .is_ok();
    if !has_auth_table {
        return; // New database or already migrated
    }

    tracing::info!("[migrate] Migrating from single-user to multi-user schema...");

    // Disable FK checks during migration (required for table recreation)
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();

    conn.execute_batch("BEGIN TRANSACTION;").unwrap();

    let result = (|| -> Result<(), rusqlite::Error> {
        // 1. Create users table from auth data (first user becomes master)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                google_id TEXT NOT NULL,
                email TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'member',
                rss_token TEXT,
                access_token TEXT,
                refresh_token TEXT,
                token_expires_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT
            );

            INSERT OR IGNORE INTO users (id, google_id, email, role, access_token, refresh_token, token_expires_at, created_at, updated_at)
            SELECT id, google_id, email, 'master', access_token, refresh_token, token_expires_at,
                   COALESCE(updated_at, datetime('now')), updated_at
            FROM auth;

            CREATE UNIQUE INDEX IF NOT EXISTS idx_users_google_id ON users(google_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_users_rss_token ON users(rss_token);",
        )?;

        // 2. Recreate sessions with user_id instead of auth_id
        conn.execute_batch(
            "CREATE TABLE sessions_new (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                expires_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            INSERT INTO sessions_new (id, user_id, expires_at, created_at)
            SELECT id, auth_id, expires_at, created_at FROM sessions;

            DROP TABLE sessions;
            ALTER TABLE sessions_new RENAME TO sessions;",
        )?;

        // 3. Create user_channels from channels (per-user preferences)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_channels (
                user_id INTEGER NOT NULL,
                channel_id TEXT NOT NULL,
                is_favorite INTEGER NOT NULL DEFAULT 0,
                show_livestreams INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (user_id, channel_id),
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
            );

            INSERT OR IGNORE INTO user_channels (user_id, channel_id, is_favorite, show_livestreams, created_at)
            SELECT u.id, c.id, c.is_favorite, c.show_livestreams, c.created_at
            FROM channels c, users u;",
        )?;

        // 4. Create user_videos from hidden videos
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_videos (
                user_id INTEGER NOT NULL,
                video_id TEXT NOT NULL,
                is_hidden INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (user_id, video_id),
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE
            );

            INSERT OR IGNORE INTO user_videos (user_id, video_id, is_hidden, created_at)
            SELECT u.id, v.id, 1, COALESCE(v.fetched_at, datetime('now'))
            FROM videos v, users u
            WHERE v.is_hidden = 1;",
        )?;

        // 5. Recreate channels without is_favorite, show_livestreams
        conn.execute_batch(
            "CREATE TABLE channels_new (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                thumbnail_url TEXT,
                upload_playlist_id TEXT,
                last_fetched_at TEXT,
                created_at TEXT NOT NULL
            );

            INSERT INTO channels_new (id, title, thumbnail_url, upload_playlist_id, last_fetched_at, created_at)
            SELECT id, title, thumbnail_url, upload_playlist_id, last_fetched_at, created_at
            FROM channels;

            DROP TABLE channels;
            ALTER TABLE channels_new RENAME TO channels;",
        )?;

        // 6. Recreate videos without is_hidden
        conn.execute_batch(
            "CREATE TABLE videos_new (
                id TEXT PRIMARY KEY,
                channel_id TEXT NOT NULL,
                title TEXT NOT NULL,
                thumbnail_url TEXT,
                published_at TEXT,
                duration TEXT,
                is_short INTEGER NOT NULL DEFAULT 0,
                is_livestream INTEGER NOT NULL DEFAULT 0,
                livestream_ended_at TEXT,
                fetched_at TEXT,
                FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
            );

            INSERT INTO videos_new (id, channel_id, title, thumbnail_url, published_at, duration, is_short, is_livestream, livestream_ended_at, fetched_at)
            SELECT id, channel_id, title, thumbnail_url, published_at, duration, is_short, is_livestream, livestream_ended_at, fetched_at
            FROM videos;

            DROP TABLE videos;
            ALTER TABLE videos_new RENAME TO videos;",
        )?;

        // 7. Recreate groups with user_id
        conn.execute_batch(
            "CREATE TABLE groups_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            INSERT INTO groups_new (id, user_id, name, sort_order, created_at)
            SELECT g.id, u.id, g.name, g.sort_order, g.created_at
            FROM groups g, users u;

            DROP TABLE groups;
            ALTER TABLE groups_new RENAME TO groups;",
        )?;

        // 8. Drop old auth table
        conn.execute_batch("DROP TABLE auth;")?;

        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT;").unwrap();
            tracing::info!("[migrate] Migration completed successfully");
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK;").unwrap();
            panic!("Migration failed: {}", e);
        }
    }

    // Re-enable FK checks
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
}

#[cfg(test)]
mod tests {
    // Database Schema Spec
    //
    // Multi-user SQLite with 8 tables. Raw SQL without ORM.
    // Master tables (shared): channels, videos
    // User tables (per-user): users, user_channels, user_videos, groups, channel_groups, sessions
    // Tables are auto-created on startup via `CREATE TABLE IF NOT EXISTS`.

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
    fn test_foreign_keys_cascade_channel_to_videos() {
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

        let expected = [
            "channel_groups", "channels", "groups", "sessions",
            "user_channels", "user_videos", "users", "videos",
        ];
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
            "idx_groups_user",
            "idx_user_channels_favorite",
            "idx_user_channels_user",
            "idx_user_videos_hidden",
            "idx_user_videos_user",
            "idx_users_google_id",
            "idx_videos_channel",
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
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'b@example.com')",
            [],
        );
        assert!(result.is_err(), "Duplicate google_id should fail");
    }

    #[test]
    fn test_idempotent_ddl() {
        let _conn1 = open_memory();
        let _conn2 = open_memory();
    }

    #[test]
    fn test_channel_subscriptions_cascade_delete() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel_subscriptions (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at)
             VALUES ('UC1', 'secret', 432000, '2024-01-01T00:00:00Z', '2024-01-06T00:00:00Z')",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM channels WHERE id = 'UC1'", []).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM channel_subscriptions WHERE channel_id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "Subscription should be deleted when channel is deleted");
    }

    #[test]
    fn test_channel_subscriptions_default_verification_status() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel_subscriptions (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at)
             VALUES ('UC1', 'secret', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let status: String = conn
            .query_row(
                "SELECT verification_status FROM channel_subscriptions WHERE channel_id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending");
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

        let (is_short, is_livestream): (i64, i64) = conn
            .query_row(
                "SELECT is_short, is_livestream FROM videos WHERE id = 'v1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(is_short, 0);
        assert_eq!(is_livestream, 0);
    }

    #[test]
    fn test_user_channels_default_values() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id) VALUES (1, 'UC1')",
            [],
        )
        .unwrap();

        let (is_favorite, show_livestreams): (i64, i64) = conn
            .query_row(
                "SELECT is_favorite, show_livestreams FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(is_favorite, 0);
        assert_eq!(show_livestreams, 0);
    }

    #[test]
    fn test_user_videos_hidden_state() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
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
        conn.execute(
            "INSERT INTO user_videos (user_id, video_id, is_hidden) VALUES (1, 'v1', 1)",
            [],
        )
        .unwrap();

        let is_hidden: i64 = conn
            .query_row(
                "SELECT is_hidden FROM user_videos WHERE user_id = 1 AND video_id = 'v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(is_hidden, 1);
    }

    #[test]
    fn test_user_channels_cascade_on_user_delete() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id) VALUES (1, 'UC1')",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM users WHERE id = 1", []).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM user_channels", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "user_channels should cascade on user delete");

        // Channel itself should remain (shared master data)
        let ch_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ch_count, 1, "Channel master data should survive user deletion");
    }

    #[test]
    fn test_user_channels_cascade_on_channel_delete() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id) VALUES (1, 'UC1')",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM channels WHERE id = 'UC1'", []).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM user_channels", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "user_channels should cascade on channel delete");
    }

    #[test]
    fn test_user_videos_cascade_on_user_delete() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
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
        conn.execute(
            "INSERT INTO user_videos (user_id, video_id, is_hidden) VALUES (1, 'v1', 1)",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM users WHERE id = 1", []).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM user_videos", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "user_videos should cascade on user delete");
    }

    #[test]
    fn test_channel_groups_cascade_on_group_delete() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Test', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO groups (user_id, name, created_at) VALUES (1, 'Group1', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let group_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', ?1)",
            [group_id],
        )
        .unwrap();

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
    fn test_sessions_cascade_on_user_delete() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        let user_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sessions (id, user_id, expires_at, created_at) VALUES ('s1', ?1, '2025-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            [user_id],
        )
        .unwrap();

        conn.execute("DELETE FROM users WHERE id = ?1", [user_id])
            .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "Session should be deleted when user is deleted");
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

    #[test]
    fn test_duplicate_composite_pk_fails() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'ch1', '2025-01-01T00:00:00Z');
             INSERT INTO groups (user_id, name, sort_order, created_at) VALUES (1, 'G1', 1, '2025-01-01T00:00:00Z');",
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

    #[test]
    fn test_user_roles() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email, role) VALUES ('g1', 'master@example.com', 'master')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (google_id, email, role) VALUES ('g2', 'admin@example.com', 'admin')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g3', 'member@example.com')",
            [],
        )
        .unwrap();

        let roles: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT role FROM users ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(roles, vec!["master", "admin", "member"]);
    }

    #[test]
    fn test_multiple_users_same_channel() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g2', 'b@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Shared', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        // Both users subscribe to the same channel
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id) VALUES (1, 'UC1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id, is_favorite) VALUES (2, 'UC1', 1)",
            [],
        )
        .unwrap();

        // User 1: not favorite, User 2: favorite
        let fav1: i64 = conn
            .query_row(
                "SELECT is_favorite FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let fav2: i64 = conn
            .query_row(
                "SELECT is_favorite FROM user_channels WHERE user_id = 2 AND channel_id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fav1, 0);
        assert_eq!(fav2, 1);
    }

    #[test]
    fn test_per_user_hidden_videos() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g2', 'b@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Ch', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC1', 'Video')",
            [],
        )
        .unwrap();

        // User 1 hides the video, User 2 does not
        conn.execute(
            "INSERT INTO user_videos (user_id, video_id, is_hidden) VALUES (1, 'v1', 1)",
            [],
        )
        .unwrap();

        // User 1: hidden
        let hidden: i64 = conn
            .query_row(
                "SELECT is_hidden FROM user_videos WHERE user_id = 1 AND video_id = 'v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hidden, 1);

        // User 2: no record = not hidden
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM user_videos WHERE user_id = 2 AND video_id = 'v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "No user_videos record means video is visible");
    }

    #[test]
    fn test_groups_require_user_id() {
        let conn = open_memory();
        // Without a valid user, group insert should fail due to FK
        let result = conn.execute(
            "INSERT INTO groups (user_id, name, created_at) VALUES (999, 'G1', '2024-01-01T00:00:00Z')",
            [],
        );
        assert!(result.is_err(), "FK constraint rejects nonexistent user_id");
    }

    #[test]
    fn test_groups_scoped_by_user() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'a@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g2', 'b@example.com')",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO groups (user_id, name, created_at) VALUES (1, 'User1 Group', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO groups (user_id, name, created_at) VALUES (2, 'User2 Group', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let user1_groups: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM groups WHERE user_id = 1")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(user1_groups, vec!["User1 Group"]);
    }

    #[test]
    fn test_migrate_from_old_schema() {
        let conn = Connection::open_in_memory().expect("Failed to open in-memory database");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        // Create old (single-user) schema
        conn.execute_batch(
            "CREATE TABLE auth (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                google_id TEXT NOT NULL,
                email TEXT NOT NULL,
                access_token TEXT,
                refresh_token TEXT,
                token_expires_at TEXT,
                updated_at TEXT
            );
            CREATE TABLE channels (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                thumbnail_url TEXT,
                upload_playlist_id TEXT,
                show_livestreams INTEGER NOT NULL DEFAULT 0,
                is_favorite INTEGER NOT NULL DEFAULT 0,
                last_fetched_at TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE videos (
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
            CREATE TABLE groups (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
            CREATE TABLE channel_groups (
                channel_id TEXT NOT NULL,
                group_id INTEGER NOT NULL,
                PRIMARY KEY (channel_id, group_id),
                FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
                FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
            );
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                auth_id INTEGER NOT NULL,
                expires_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (auth_id) REFERENCES auth(id) ON DELETE CASCADE
            );",
        ).unwrap();

        // Insert old data
        conn.execute_batch(
            "INSERT INTO auth (google_id, email, access_token, refresh_token, token_expires_at, updated_at)
             VALUES ('g1', 'test@example.com', 'at', 'rt', '2026-01-01T00:00:00Z', '2025-01-01T00:00:00Z');
             INSERT INTO channels (id, title, show_livestreams, is_favorite, created_at)
             VALUES ('UC1', 'Ch1', 1, 1, '2024-01-01T00:00:00Z');
             INSERT INTO videos (id, channel_id, title, is_hidden) VALUES ('v1', 'UC1', 'Visible', 0);
             INSERT INTO videos (id, channel_id, title, is_hidden) VALUES ('v2', 'UC1', 'Hidden', 1);
             INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 0, '2024-01-01T00:00:00Z');
             INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1);
             INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES ('s1', 1, '2026-01-01T00:00:00Z', '2025-01-01T00:00:00Z');",
        ).unwrap();

        // Run migration + create_tables
        migrate(&conn);
        create_tables(&conn);

        // Verify: auth table gone, users table exists with master role
        assert!(conn.prepare("SELECT 1 FROM auth").is_err(), "auth table should be dropped");
        let (role, email): (String, String) = conn
            .query_row("SELECT role, email FROM users WHERE id = 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!(role, "master");
        assert_eq!(email, "test@example.com");

        // Verify: user_channels migrated with preferences
        let (is_fav, show_live): (i64, i64) = conn
            .query_row(
                "SELECT is_favorite, show_livestreams FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(is_fav, 1, "is_favorite should migrate from channels");
        assert_eq!(show_live, 1, "show_livestreams should migrate from channels");

        // Verify: hidden video migrated to user_videos
        let hidden_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM user_videos WHERE user_id = 1 AND is_hidden = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(hidden_count, 1, "Hidden video should be in user_videos");

        // Verify: channels table no longer has is_favorite/show_livestreams
        assert!(conn.prepare("SELECT is_favorite FROM channels").is_err(), "channels should not have is_favorite");
        assert!(conn.prepare("SELECT show_livestreams FROM channels").is_err(), "channels should not have show_livestreams");

        // Verify: videos table no longer has is_hidden
        assert!(conn.prepare("SELECT is_hidden FROM videos").is_err(), "videos should not have is_hidden");

        // Verify: sessions migrated (user_id, not auth_id)
        let user_id: i64 = conn
            .query_row("SELECT user_id FROM sessions WHERE id = 's1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_id, 1);

        // Verify: groups have user_id
        let group_user: i64 = conn
            .query_row("SELECT user_id FROM groups WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(group_user, 1);
    }

    #[test]
    fn test_rss_token_unique_index() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email, rss_token) VALUES ('g1', 'a@example.com', 'token-123')",
            [],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO users (google_id, email, rss_token) VALUES ('g2', 'b@example.com', 'token-123')",
            [],
        );
        assert!(result.is_err(), "Duplicate rss_token should fail");
    }
}
