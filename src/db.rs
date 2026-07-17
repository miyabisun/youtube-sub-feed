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
    add_user_channels_hide_shorts(&conn);
    drop_videos_thumbnail_url(&conn);
    add_videos_is_members_only(&conn);
    migrate_timestamps_to_unix(&conn);
    add_videos_details_checked_at(&conn);
    decode_video_titles_xml_entities(&conn);
    drop_users_oauth_token_columns(&conn);
    add_users_email_unique_index(&conn);

    conn
}

/// Add the per-user, per-channel Shorts suppression preference.
///
/// Existing subscriptions keep showing Shorts because the column defaults to
/// zero. Idempotent so it is safe to run at every startup.
fn add_user_channels_hide_shorts(conn: &Connection) {
    if column_exists(conn, "user_channels", "hide_shorts") {
        return;
    }
    match conn.execute(
        "ALTER TABLE user_channels ADD COLUMN hide_shorts INTEGER NOT NULL DEFAULT 0",
        [],
    ) {
        Ok(_) => tracing::info!("[migrate] Added user_channels.hide_shorts column"),
        Err(e) => tracing::warn!(
            "[migrate] Failed to add user_channels.hide_shorts column: {}",
            e
        ),
    }
}

/// Track when a video's details were last fetched from the YouTube Data API.
///
/// NULL means "never attempted": the enrichment backfill re-queries only those
/// rows, so videos deleted from YouTube (absent from API responses) stop being
/// re-queried once their batch succeeds. Runs after migrate_timestamps_to_unix
/// because that migration rebuilds `videos` without this column. Idempotent.
fn add_videos_details_checked_at(conn: &Connection) {
    if column_exists(conn, "videos", "details_checked_at") {
        return;
    }
    match conn.execute(
        "ALTER TABLE videos ADD COLUMN details_checked_at INTEGER",
        [],
    ) {
        Ok(_) => tracing::info!("[migrate] Added videos.details_checked_at column"),
        Err(e) => tracing::warn!(
            "[migrate] Failed to add videos.details_checked_at column: {}",
            e
        ),
    }
}

/// One-shot migration: decode XML entities in legacy `videos.title` rows.
///
/// Pre-fix WebSub callbacks stored Atom titles verbatim (e.g. "S&amp;P500"),
/// which caused the browser to render the literal "S&amp;P500" because the
/// frontend re-escapes the `&`. New rows are now decoded at ingest in
/// `websub::atom`, so this pass only exists to clean up rows from before
/// that fix.
///
/// Returns the number of rows actually rewritten — primarily to let tests
/// assert that clean rows are skipped, which is what makes the WHERE filter
/// load-bearing rather than cosmetic.
///
/// Idempotent: the `WHERE` predicate skips rows that no longer carry an
/// `&...;` substring, and `REPLACE()` on already-decoded text is a no-op.
/// Cheap enough to run on every startup; the WHERE filter keeps the work
/// proportional to the number of dirty rows once the backlog is drained.
///
/// Numeric character references (`&#39;`, `&#x27;`) are intentionally not
/// handled here: SQLite has no built-in regex and these are rare enough in
/// real YouTube feeds that adding a Rust-side scan is not worth the cost.
/// New rows go through `websub::atom::decode_xml_entities`, which does
/// handle them.
///
/// Keep the named-entity set in sync with `websub::atom::decode_xml_entities`.
fn decode_video_titles_xml_entities(conn: &Connection) -> usize {
    match conn.execute(
        "UPDATE videos
         SET title = REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(
             title,
             '&lt;', '<'),
             '&gt;', '>'),
             '&quot;', '\"'),
             '&apos;', ''''),
             '&amp;', '&')
         WHERE title LIKE '%&%;%'",
        [],
    ) {
        Ok(updated) => {
            if updated > 0 {
                tracing::info!(
                    "[migrate] Decoded XML entities in {} video title(s)",
                    updated
                );
            }
            updated
        }
        Err(e) => {
            tracing::warn!("[migrate] Failed to decode video titles: {}", e);
            0
        }
    }
}

/// One-shot migration: add `videos.is_members_only` to legacy databases.
///
/// Rows default to 0 (not members-only); the next periodic refresh fills in
/// the truth via the channel's UUMO playlist. Idempotent.
fn add_videos_is_members_only(conn: &Connection) {
    if column_exists(conn, "videos", "is_members_only") {
        return;
    }
    match conn.execute(
        "ALTER TABLE videos ADD COLUMN is_members_only INTEGER NOT NULL DEFAULT 0",
        [],
    ) {
        Ok(_) => tracing::info!("[migrate] Added videos.is_members_only column"),
        Err(e) => tracing::warn!("[migrate] Failed to add videos.is_members_only: {}", e),
    }
}

/// One-shot migration: remove the obsolete `videos.thumbnail_url` column.
///
/// Thumbnails are derived from the video ID on the client side, so storing
/// the URL adds no information and creates failure modes (NULL columns when
/// videos arrive via WebSub push that doesn't carry thumbnail metadata).
/// Idempotent: a no-op once the column has been dropped.
fn drop_videos_thumbnail_url(conn: &Connection) {
    let column_exists = column_exists(conn, "videos", "thumbnail_url");
    if !column_exists {
        return;
    }
    match conn.execute("ALTER TABLE videos DROP COLUMN thumbnail_url", []) {
        Ok(_) => tracing::info!("[migrate] Dropped obsolete videos.thumbnail_url column"),
        Err(e) => tracing::warn!("[migrate] Failed to drop videos.thumbnail_url: {}", e),
    }
}

/// One-shot migration: drop OAuth token columns from legacy `users` tables.
///
/// Before the OAuth-removal refactor, users had `access_token`, `refresh_token`,
/// and `token_expires_at` columns. These are now obsolete: the server never holds
/// any OAuth tokens. Idempotent: a no-op if the columns are already absent.
fn drop_users_oauth_token_columns(conn: &Connection) {
    for col in &["access_token", "refresh_token", "token_expires_at"] {
        if column_exists(conn, "users", col) {
            match conn.execute(&format!("ALTER TABLE users DROP COLUMN {}", col), []) {
                Ok(_) => tracing::info!("[migrate] Dropped obsolete users.{} column", col),
                Err(e) => tracing::warn!("[migrate] Failed to drop users.{}: {}", col, e),
            }
        }
    }
}

/// One-shot migration: add a UNIQUE index on users.email (replacing the old
/// idx_users_google_id index). Email is now the primary user identifier since
/// Cloudflare Access provides it via the `Cf-Access-Authenticated-User-Email`
/// header. Idempotent: a no-op if the index already exists.
fn add_users_email_unique_index(conn: &Connection) {
    // Drop the old google_id unique index if it exists (it would conflict with
    // nullable google_id values from newly created users).
    let has_old_idx = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'idx_users_google_id' LIMIT 1")
        .ok()
        .and_then(|mut s| s.query_row([], |_| Ok(true)).ok())
        .unwrap_or(false);
    if has_old_idx {
        let _ = conn.execute("DROP INDEX IF EXISTS idx_users_google_id", []);
        tracing::info!("[migrate] Dropped obsolete idx_users_google_id index");
    }

    // Add email unique index if missing.
    let has_email_idx = conn
        .prepare(
            "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'idx_users_email' LIMIT 1",
        )
        .ok()
        .and_then(|mut s| s.query_row([], |_| Ok(true)).ok())
        .unwrap_or(false);
    if !has_email_idx {
        match conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email ON users(email)",
            [],
        ) {
            Ok(_) => tracing::info!("[migrate] Created idx_users_email index"),
            Err(e) => tracing::warn!("[migrate] Failed to create idx_users_email: {}", e),
        }
    }
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({})", table);
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let Ok(names) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return false;
    };
    // `names` borrows `stmt` (MappedRows<'_, _>). Returning the chained
    // expression directly defers `stmt`'s drop past `names`'s borrow window
    // and rustc rejects it. Binding to `found` finishes the borrow before
    // the return value is produced.
    #[allow(clippy::let_and_return)]
    let found = names.filter_map(|r| r.ok()).any(|n| n == column);
    found
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
            google_id TEXT,
            email TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'member',
            rss_token TEXT,
            created_at INTEGER DEFAULT (unixepoch()),
            updated_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS channels (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            thumbnail_url TEXT,
            upload_playlist_id TEXT,
            last_fetched_at INTEGER,
            created_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS videos (
            id TEXT PRIMARY KEY,
            channel_id TEXT NOT NULL,
            title TEXT NOT NULL,
            published_at INTEGER,
            duration TEXT,
            is_short INTEGER NOT NULL DEFAULT 0,
            is_livestream INTEGER NOT NULL DEFAULT 0,
            is_members_only INTEGER NOT NULL DEFAULT 0,
            livestream_ended_at INTEGER,
            fetched_at INTEGER,
            details_checked_at INTEGER,
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS user_channels (
            user_id INTEGER NOT NULL,
            channel_id TEXT NOT NULL,
            is_favorite INTEGER NOT NULL DEFAULT 0,
            show_livestreams INTEGER NOT NULL DEFAULT 0,
            hide_shorts INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER DEFAULT (unixepoch()),
            PRIMARY KEY (user_id, channel_id),
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS user_videos (
            user_id INTEGER NOT NULL,
            video_id TEXT NOT NULL,
            is_hidden INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER DEFAULT (unixepoch()),
            PRIMARY KEY (user_id, video_id),
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
            FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS channel_groups (
            channel_id TEXT NOT NULL,
            group_id INTEGER NOT NULL,
            PRIMARY KEY (channel_id, group_id),
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
            FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS channel_subscriptions (
            channel_id TEXT PRIMARY KEY,
            hub_secret TEXT NOT NULL,
            lease_seconds INTEGER NOT NULL DEFAULT 0,
            subscribed_at INTEGER,
            expires_at INTEGER,
            verification_status TEXT NOT NULL DEFAULT 'pending',
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email ON users(email);
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

/// Convert legacy RFC 3339 TEXT timestamps to Unix seconds and rebuild the
/// timestamp-bearing tables with INTEGER affinity. Values without an explicit
/// offset are ambiguous and become NULL rather than being guessed as UTC/JST.
fn migrate_timestamps_to_unix(conn: &Connection) {
    const CORE_TIMESTAMPS: &[(&str, &[&str])] = &[
        ("users", &["created_at", "updated_at"]),
        ("channels", &["last_fetched_at", "created_at"]),
        (
            "videos",
            &["published_at", "livestream_ended_at", "fetched_at"],
        ),
        ("user_channels", &["created_at"]),
        ("user_videos", &["created_at"]),
        ("groups", &["created_at"]),
        ("channel_subscriptions", &["subscribed_at", "expires_at"]),
    ];
    let needs_rebuild = CORE_TIMESTAMPS.iter().any(|(table, columns)| {
        columns.iter().any(|column| {
            column_declaration(conn, table, column)
                .map(|(kind, not_null)| kind != "INTEGER" || not_null)
                .unwrap_or(true)
        })
    });
    if !needs_rebuild {
        normalize_timestamp_storage(conn, CORE_TIMESTAMPS);
        migrate_sessions_to_unix(conn);
        return;
    }

    tracing::info!("[migrate] Converting SQLite timestamps to Unix seconds");
    conn.execute_batch("PRAGMA foreign_keys = OFF; BEGIN IMMEDIATE;")
        .expect("Failed to start timestamp migration");

    let result = conn.execute_batch(
        "DROP INDEX IF EXISTS idx_users_email;
         DROP INDEX IF EXISTS idx_users_rss_token;
         DROP INDEX IF EXISTS idx_videos_published;
         DROP INDEX IF EXISTS idx_videos_channel;
         DROP INDEX IF EXISTS idx_user_channels_user;
         DROP INDEX IF EXISTS idx_user_channels_favorite;
         DROP INDEX IF EXISTS idx_user_videos_user;
         DROP INDEX IF EXISTS idx_user_videos_hidden;
         DROP INDEX IF EXISTS idx_groups_user;
         DROP INDEX IF EXISTS idx_channel_subscriptions_expires;

         CREATE TABLE users_unix (
            id INTEGER PRIMARY KEY AUTOINCREMENT, google_id TEXT, email TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'member', rss_token TEXT,
            created_at INTEGER DEFAULT (unixepoch()), updated_at INTEGER);
         INSERT INTO users_unix
         SELECT id, google_id, email, role, rss_token,
            CASE WHEN typeof(created_at)='integer' THEN created_at WHEN instr(created_at,'T')>0 AND (substr(trim(created_at),-1)='Z' OR substr(trim(created_at),-6,1) IN ('+','-')) THEN unixepoch(created_at) END,
            CASE WHEN typeof(updated_at)='integer' THEN updated_at WHEN instr(updated_at,'T')>0 AND (substr(trim(updated_at),-1)='Z' OR substr(trim(updated_at),-6,1) IN ('+','-')) THEN unixepoch(updated_at) END
         FROM users;

         CREATE TABLE channels_unix (
            id TEXT PRIMARY KEY, title TEXT NOT NULL, thumbnail_url TEXT,
            upload_playlist_id TEXT, last_fetched_at INTEGER, created_at INTEGER);
         INSERT INTO channels_unix
         SELECT id,title,thumbnail_url,upload_playlist_id,
            CASE WHEN typeof(last_fetched_at)='integer' THEN last_fetched_at WHEN instr(last_fetched_at,'T')>0 AND (substr(trim(last_fetched_at),-1)='Z' OR substr(trim(last_fetched_at),-6,1) IN ('+','-')) THEN unixepoch(last_fetched_at) END,
            CASE WHEN typeof(created_at)='integer' THEN created_at WHEN instr(created_at,'T')>0 AND (substr(trim(created_at),-1)='Z' OR substr(trim(created_at),-6,1) IN ('+','-')) THEN unixepoch(created_at) END
         FROM channels;

         CREATE TABLE videos_unix (
            id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, title TEXT NOT NULL,
            published_at INTEGER, duration TEXT, is_short INTEGER NOT NULL DEFAULT 0,
            is_livestream INTEGER NOT NULL DEFAULT 0, is_members_only INTEGER NOT NULL DEFAULT 0,
            livestream_ended_at INTEGER, fetched_at INTEGER,
            FOREIGN KEY(channel_id) REFERENCES channels(id) ON DELETE CASCADE);
         INSERT INTO videos_unix
         SELECT id,channel_id,title,
            CASE WHEN typeof(published_at)='integer' THEN published_at WHEN instr(published_at,'T')>0 AND (substr(trim(published_at),-1)='Z' OR substr(trim(published_at),-6,1) IN ('+','-')) THEN unixepoch(published_at) END,
            duration,is_short,is_livestream,is_members_only,
            CASE WHEN typeof(livestream_ended_at)='integer' THEN livestream_ended_at WHEN instr(livestream_ended_at,'T')>0 AND (substr(trim(livestream_ended_at),-1)='Z' OR substr(trim(livestream_ended_at),-6,1) IN ('+','-')) THEN unixepoch(livestream_ended_at) END,
            CASE WHEN typeof(fetched_at)='integer' THEN fetched_at WHEN instr(fetched_at,'T')>0 AND (substr(trim(fetched_at),-1)='Z' OR substr(trim(fetched_at),-6,1) IN ('+','-')) THEN unixepoch(fetched_at) END
         FROM videos;

         CREATE TABLE user_channels_unix (
            user_id INTEGER NOT NULL, channel_id TEXT NOT NULL, is_favorite INTEGER NOT NULL DEFAULT 0,
            show_livestreams INTEGER NOT NULL DEFAULT 0, hide_shorts INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER DEFAULT (unixepoch()),
            PRIMARY KEY(user_id,channel_id), FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
            FOREIGN KEY(channel_id) REFERENCES channels(id) ON DELETE CASCADE);
         INSERT INTO user_channels_unix
         SELECT user_id,channel_id,is_favorite,show_livestreams,hide_shorts,
            CASE WHEN typeof(created_at)='integer' THEN created_at WHEN instr(created_at,'T')>0 AND (substr(trim(created_at),-1)='Z' OR substr(trim(created_at),-6,1) IN ('+','-')) THEN unixepoch(created_at) END
         FROM user_channels;

         CREATE TABLE user_videos_unix (
            user_id INTEGER NOT NULL, video_id TEXT NOT NULL, is_hidden INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER DEFAULT (unixepoch()), PRIMARY KEY(user_id,video_id),
            FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
            FOREIGN KEY(video_id) REFERENCES videos(id) ON DELETE CASCADE);
         INSERT INTO user_videos_unix
         SELECT user_id,video_id,is_hidden,
            CASE WHEN typeof(created_at)='integer' THEN created_at WHEN instr(created_at,'T')>0 AND (substr(trim(created_at),-1)='Z' OR substr(trim(created_at),-6,1) IN ('+','-')) THEN unixepoch(created_at) END
         FROM user_videos;

         CREATE TABLE groups_unix (
            id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, name TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0, created_at INTEGER,
            FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE);
         INSERT INTO groups_unix
         SELECT id,user_id,name,sort_order,
            CASE WHEN typeof(created_at)='integer' THEN created_at WHEN instr(created_at,'T')>0 AND (substr(trim(created_at),-1)='Z' OR substr(trim(created_at),-6,1) IN ('+','-')) THEN unixepoch(created_at) END
         FROM groups;

         CREATE TABLE channel_subscriptions_unix (
            channel_id TEXT PRIMARY KEY, hub_secret TEXT NOT NULL, lease_seconds INTEGER NOT NULL DEFAULT 0,
            subscribed_at INTEGER, expires_at INTEGER, verification_status TEXT NOT NULL DEFAULT 'pending',
            FOREIGN KEY(channel_id) REFERENCES channels(id) ON DELETE CASCADE);
         INSERT INTO channel_subscriptions_unix
         SELECT channel_id,hub_secret,lease_seconds,
            CASE WHEN typeof(subscribed_at)='integer' THEN subscribed_at WHEN instr(subscribed_at,'T')>0 AND (substr(trim(subscribed_at),-1)='Z' OR substr(trim(subscribed_at),-6,1) IN ('+','-')) THEN unixepoch(subscribed_at) END,
            CASE WHEN typeof(expires_at)='integer' THEN expires_at WHEN instr(expires_at,'T')>0 AND (substr(trim(expires_at),-1)='Z' OR substr(trim(expires_at),-6,1) IN ('+','-')) THEN unixepoch(expires_at) END,
            verification_status FROM channel_subscriptions;

         DROP TABLE channel_subscriptions; DROP TABLE groups; DROP TABLE user_videos;
         DROP TABLE user_channels; DROP TABLE videos; DROP TABLE channels; DROP TABLE users;
         ALTER TABLE users_unix RENAME TO users;
         ALTER TABLE channels_unix RENAME TO channels;
         ALTER TABLE videos_unix RENAME TO videos;
         ALTER TABLE user_channels_unix RENAME TO user_channels;
         ALTER TABLE user_videos_unix RENAME TO user_videos;
         ALTER TABLE groups_unix RENAME TO groups;
         ALTER TABLE channel_subscriptions_unix RENAME TO channel_subscriptions;
         COMMIT;",
    );
    if let Err(error) = result {
        let _ = conn.execute_batch("ROLLBACK;");
        panic!("Timestamp migration failed: {error}");
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("Failed to restore foreign keys");
    create_tables(conn);
    normalize_timestamp_storage(conn, CORE_TIMESTAMPS);
    migrate_sessions_to_unix(conn);
}

fn column_declaration(conn: &Connection, table: &str, column: &str) -> Option<(String, bool)> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).ok()?;
    let found = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? != 0,
            ))
        })
        .ok()?
        .filter_map(Result::ok)
        .find_map(|(name, kind, not_null)| (name == column).then_some((kind, not_null)));
    found
}

fn timestamp_conversion(column: &str) -> String {
    format!(
        "CASE WHEN typeof({column})='integer' THEN {column} \
         WHEN instr({column},'T')>0 AND (substr(trim({column}),-1)='Z' OR substr(trim({column}),-6,1) IN ('+','-')) \
         THEN unixepoch({column}) END"
    )
}

fn normalize_timestamp_storage(conn: &Connection, tables: &[(&str, &[&str])]) {
    for (table, columns) in tables {
        let assignments = columns
            .iter()
            .map(|column| format!("{column}={}", timestamp_conversion(column)))
            .collect::<Vec<_>>()
            .join(",");
        conn.execute_batch(&format!("UPDATE {table} SET {assignments};"))
            .unwrap_or_else(|error| panic!("Failed to normalize {table} timestamps: {error}"));
    }
}

fn migrate_sessions_to_unix(conn: &Connection) {
    if conn.prepare("SELECT 1 FROM sessions LIMIT 0").is_err() {
        return;
    }
    let needs_rebuild = ["expires_at", "created_at"].iter().any(|column| {
        column_declaration(conn, "sessions", column)
            .map(|(kind, not_null)| kind != "INTEGER" || not_null)
            .unwrap_or(true)
    });
    if !needs_rebuild {
        conn.execute_batch(&format!(
            "UPDATE sessions SET expires_at={}, created_at={};",
            timestamp_conversion("expires_at"),
            timestamp_conversion("created_at")
        ))
        .expect("Failed to normalize session timestamps");
        return;
    }
    conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE;")
        .expect("Failed to start session timestamp migration");
    let result = conn.execute_batch(&format!(
        "CREATE TABLE sessions_unix (
            id TEXT PRIMARY KEY, user_id INTEGER NOT NULL, expires_at INTEGER, created_at INTEGER,
            FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE);
         INSERT INTO sessions_unix SELECT id,user_id,{},{} FROM sessions;
         DROP TABLE sessions;
         ALTER TABLE sessions_unix RENAME TO sessions;
         COMMIT;",
        timestamp_conversion("expires_at"),
        timestamp_conversion("created_at")
    ));
    if let Err(error) = result {
        let _ = conn.execute_batch("ROLLBACK;");
        panic!("Session timestamp migration failed: {error}");
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .expect("Failed to restore foreign keys");
}

fn migrate(conn: &Connection) {
    let has_auth_table = conn.prepare("SELECT 1 FROM auth LIMIT 0").is_ok();
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
                google_id TEXT,
                email TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'member',
                rss_token TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at INTEGER
            );

            INSERT OR IGNORE INTO users (id, google_id, email, role, created_at, updated_at)
            SELECT id, google_id, email, 'master',
                   COALESCE(updated_at, unixepoch()), updated_at
            FROM auth;

            CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email ON users(email);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_users_rss_token ON users(rss_token);",
        )?;

        // 2. Recreate sessions with user_id instead of auth_id
        conn.execute_batch(
            "CREATE TABLE sessions_new (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                expires_at INTEGER,
                created_at INTEGER,
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
                hide_shorts INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
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
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (user_id, video_id),
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE
            );

            INSERT OR IGNORE INTO user_videos (user_id, video_id, is_hidden, created_at)
            SELECT u.id, v.id, 1, COALESCE(v.fetched_at, unixepoch())
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
                last_fetched_at INTEGER,
                created_at INTEGER
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
                livestream_ended_at INTEGER,
                fetched_at INTEGER,
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
                created_at INTEGER,
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

        // sessions テーブルは OAuth 撤去・Cloudflare Access 移行に伴い削除された。
        // 新規 DB には sessions テーブルは存在しない。
        let expected = [
            "channel_groups",
            "channel_subscriptions",
            "channels",
            "groups",
            "user_channels",
            "user_videos",
            "users",
            "videos",
        ];
        for name in &expected {
            assert!(
                tables.contains(&name.to_string()),
                "Table '{}' not found",
                name
            );
        }

        // Cloudflare Access 委譲後、Cookie セッションは廃止。
        // 新規スキーマに sessions テーブルが混入していないことを保証する。
        assert!(
            !tables.contains(&"sessions".to_string()),
            "sessions table must not exist in new schema (OAuth/cookie sessions are abolished)"
        );
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

        // Full match (not a subset): every index create_tables declares must be
        // present, and no extra ones. This catches both a dropped index and a
        // stale expectation list. idx_users_rss_token and
        // idx_channel_subscriptions_expires were previously missing here.
        let expected = vec![
            "idx_channel_subscriptions_expires",
            "idx_groups_user",
            "idx_user_channels_favorite",
            "idx_user_channels_user",
            "idx_user_videos_hidden",
            "idx_user_videos_user",
            "idx_users_email",
            "idx_users_rss_token",
            "idx_videos_channel",
            "idx_videos_published",
        ];
        assert_eq!(
            indexes, expected,
            "index set must match create_tables exactly"
        );
    }

    #[test]
    fn email_unique_index_prevents_duplicate_email() {
        // Email is now the primary user identifier (replacing google_id).
        let conn = open_memory();
        conn.execute("INSERT INTO users (email) VALUES ('a@example.com')", [])
            .unwrap();
        let result = conn.execute("INSERT INTO users (email) VALUES ('a@example.com')", []);
        assert!(result.is_err(), "Duplicate email should fail");
    }

    #[test]
    fn google_id_is_nullable_in_new_schema() {
        // google_id is nullable since Cloudflare Access identifies by email.
        let conn = open_memory();
        let result = conn.execute(
            "INSERT INTO users (email) VALUES ('no_google@example.com')",
            [],
        );
        assert!(
            result.is_ok(),
            "Inserting user without google_id should succeed"
        );
    }

    #[test]
    fn create_tables_is_idempotent_and_preserves_existing_data() {
        // open_memory already ran create_tables once. Insert a row, then run the
        // DDL again on the *same* connection: `CREATE TABLE IF NOT EXISTS` and
        // `CREATE ... INDEX IF NOT EXISTS` must be no-ops that leave data intact.
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Keep', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        super::create_tables(&conn);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM channels WHERE id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "re-applying create_tables must not drop existing data"
        );
    }

    #[test]
    fn test_drop_videos_thumbnail_url_when_present() {
        // Simulate a legacy DB that still carries the obsolete column.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE channels (id TEXT PRIMARY KEY, title TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE videos (
               id TEXT PRIMARY KEY,
               channel_id TEXT NOT NULL,
               title TEXT NOT NULL,
               thumbnail_url TEXT,
               fetched_at TEXT
             );",
        )
        .unwrap();
        assert!(
            super::column_exists(&conn, "videos", "thumbnail_url"),
            "precondition"
        );

        super::drop_videos_thumbnail_url(&conn);

        assert!(
            !super::column_exists(&conn, "videos", "thumbnail_url"),
            "thumbnail_url should have been dropped"
        );
    }

    #[test]
    fn test_add_videos_is_members_only_when_missing() {
        // Legacy DB without the new column.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE channels (id TEXT PRIMARY KEY, title TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE videos (
               id TEXT PRIMARY KEY,
               channel_id TEXT NOT NULL,
               title TEXT NOT NULL,
               fetched_at TEXT
             );",
        )
        .unwrap();
        assert!(
            !super::column_exists(&conn, "videos", "is_members_only"),
            "precondition"
        );

        super::add_videos_is_members_only(&conn);

        assert!(super::column_exists(&conn, "videos", "is_members_only"));
    }

    #[test]
    fn test_add_videos_is_members_only_is_idempotent() {
        // Fresh DB already has the column from create_tables.
        let conn = open_memory();
        assert!(
            super::column_exists(&conn, "videos", "is_members_only"),
            "precondition"
        );
        super::add_videos_is_members_only(&conn);
        assert!(super::column_exists(&conn, "videos", "is_members_only"));
    }

    #[test]
    fn test_drop_videos_thumbnail_url_is_idempotent() {
        // No column: should be a no-op without errors.
        let conn = open_memory();
        assert!(
            !super::column_exists(&conn, "videos", "thumbnail_url"),
            "precondition"
        );

        super::drop_videos_thumbnail_url(&conn);

        assert!(!super::column_exists(&conn, "videos", "thumbnail_url"));
    }

    #[test]
    fn test_decode_video_titles_xml_entities_fixes_legacy_rows() {
        // Pre-fix WebSub callbacks stored Atom-encoded titles verbatim, so
        // legacy DBs carry rows like "S&amp;P500". This migration must
        // decode them in place.
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Ch', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO videos (id, channel_id, title) VALUES ('v_amp', 'UC1', 'S&amp;P500の解説');
             INSERT INTO videos (id, channel_id, title) VALUES ('v_all', 'UC1', 'a&lt;b&gt;c&quot;d&apos;e&amp;f');
             INSERT INTO videos (id, channel_id, title) VALUES ('v_clean', 'UC1', '普通のタイトル');",
        )
        .unwrap();

        super::decode_video_titles_xml_entities(&conn);

        let lookup = |id: &str| -> String {
            conn.query_row("SELECT title FROM videos WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap()
        };
        assert_eq!(lookup("v_amp"), "S&P500の解説");
        assert_eq!(lookup("v_all"), "a<b>c\"d'e&f");
        assert_eq!(lookup("v_clean"), "普通のタイトル");
    }

    #[test]
    fn test_decode_video_titles_xml_entities_is_idempotent() {
        // Running the migration twice must not corrupt already-decoded text.
        // This is what makes it safe to call unconditionally on every startup.
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Ch', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC1', 'S&amp;P500')",
            [],
        )
        .unwrap();

        let first = super::decode_video_titles_xml_entities(&conn);
        let second = super::decode_video_titles_xml_entities(&conn);

        assert_eq!(first, 1, "First pass should rewrite the dirty row");
        assert_eq!(
            second, 0,
            "Second pass must be a no-op on already-decoded text"
        );

        let title: String = conn
            .query_row("SELECT title FROM videos WHERE id = 'v1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(title, "S&P500");
    }

    #[test]
    fn test_decode_video_titles_xml_entities_preserves_literal_ampersand() {
        // "AT&T" contains a literal '&' that is not part of any XML entity.
        // The WHERE filter and REPLACE chain must leave such rows untouched.
        let conn = open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Ch', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title) VALUES ('v', 'UC1', 'AT&T 決算')",
            [],
        )
        .unwrap();

        let updated = super::decode_video_titles_xml_entities(&conn);
        assert_eq!(updated, 0, "Row without an entity must not be touched");

        let title: String = conn
            .query_row("SELECT title FROM videos WHERE id = 'v'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(title, "AT&T 決算");
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

        conn.execute("DELETE FROM channels WHERE id = 'UC1'", [])
            .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM channel_subscriptions WHERE channel_id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "Subscription should be deleted when channel is deleted"
        );
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

        let (is_short, is_livestream, is_members_only): (i64, i64, i64) = conn
            .query_row(
                "SELECT is_short, is_livestream, is_members_only FROM videos WHERE id = 'v1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(is_short, 0);
        assert_eq!(is_livestream, 0);
        assert_eq!(
            is_members_only, 0,
            "is_members_only must default to 0 (not members-only) on insert"
        );
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

        let (is_favorite, show_livestreams, hide_shorts): (i64, i64, i64) = conn
            .query_row(
                "SELECT is_favorite, show_livestreams, hide_shorts FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(is_favorite, 0);
        assert_eq!(show_livestreams, 0);
        assert_eq!(hide_shorts, 0);
    }

    #[test]
    fn add_user_channels_hide_shorts_preserves_existing_subscriptions() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE user_channels (
                user_id INTEGER NOT NULL,
                channel_id TEXT NOT NULL,
                is_favorite INTEGER NOT NULL DEFAULT 0,
                show_livestreams INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER,
                PRIMARY KEY (user_id, channel_id)
            );
            INSERT INTO user_channels
                (user_id, channel_id, is_favorite, show_livestreams, created_at)
            VALUES (7, 'UClegacy', 1, 1, 123);",
        )
        .unwrap();

        super::add_user_channels_hide_shorts(&conn);
        super::add_user_channels_hide_shorts(&conn);

        let row: (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT is_favorite, show_livestreams, hide_shorts, created_at
                 FROM user_channels WHERE user_id = 7 AND channel_id = 'UClegacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, (1, 1, 0, 123));
    }

    #[test]
    fn add_videos_details_checked_at_defaults_to_null_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE videos (
                id TEXT PRIMARY KEY,
                channel_id TEXT NOT NULL,
                title TEXT NOT NULL,
                duration TEXT,
                is_short INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO videos (id, channel_id, title) VALUES ('v_legacy', 'UC1', 'T');",
        )
        .unwrap();

        super::add_videos_details_checked_at(&conn);
        super::add_videos_details_checked_at(&conn);

        // NULL marks the row as "never enriched", making it a backfill target.
        let checked_at: Option<i64> = conn
            .query_row(
                "SELECT details_checked_at FROM videos WHERE id = 'v_legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(checked_at, None);
    }

    #[test]
    fn test_user_videos_defaults_is_hidden_to_zero() {
        // When a user_videos row is created without an explicit is_hidden, the
        // column must default to 0 (visible). Per-user hide/unhide behaviour is
        // covered by test_per_user_hidden_videos.
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
        // Note: is_hidden intentionally omitted.
        conn.execute(
            "INSERT INTO user_videos (user_id, video_id) VALUES (1, 'v1')",
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
        assert_eq!(is_hidden, 0, "is_hidden must default to 0 when omitted");
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
        assert_eq!(
            ch_count, 1,
            "Channel master data should survive user deletion"
        );
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

        conn.execute("DELETE FROM channels WHERE id = 'UC1'", [])
            .unwrap();

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
        assert!(
            val.is_none(),
            "last_fetched_at should be NULL before first fetch"
        );
    }

    #[test]
    fn test_duplicate_id_fails_primary_key() {
        let conn = open_memory();
        let insert = "INSERT INTO channels (id, title, created_at) VALUES ('UC3', 'ch', '2025-01-01T00:00:00Z')";
        conn.execute(insert, []).unwrap();
        assert!(
            conn.execute(insert, []).is_err(),
            "duplicate PK insert should fail"
        );
    }

    #[test]
    fn test_upsert_updates_utf8_title() {
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
            "INSERT INTO videos (id, channel_id, title, fetched_at)
             VALUES ('vid1', 'UC1', '新しいタイトル', '2025-06-01T00:00:00Z')
             ON CONFLICT(id) DO UPDATE SET title = excluded.title
             WHERE title != excluded.title",
            [],
        )
        .unwrap();

        let title: String = conn
            .query_row("SELECT title FROM videos WHERE id = 'vid1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            title, "新しいタイトル",
            "UPSERT should update title to follow author edits"
        );
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

        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)",
            [],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)",
            [],
        );
        assert!(
            result.is_err(),
            "composite PK (channel_id, group_id) prevents duplicates"
        );
    }

    #[test]
    fn test_insert_video_with_nonexistent_channel_fails_fk() {
        let conn = open_memory();
        let result = conn.execute(
            "INSERT INTO videos (id, channel_id, title) VALUES ('vid1', 'UC_nonexistent', 'v')",
            [],
        );
        assert!(
            result.is_err(),
            "FK constraint rejects nonexistent channel_id"
        );
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
            let mut stmt = conn.prepare("SELECT role FROM users ORDER BY id").unwrap();
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
            -- sessions テーブルも旧スキーマに存在したが、migrate() がリネームする
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                auth_id INTEGER NOT NULL,
                expires_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (auth_id) REFERENCES auth(id) ON DELETE CASCADE
            );",
        )
        .unwrap();

        // Insert old data (sessions は旧スキーマの auth_id 参照)
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

        // Run the same migration chain as open().
        migrate(&conn);
        create_tables(&conn);
        add_user_channels_hide_shorts(&conn);
        drop_videos_thumbnail_url(&conn);
        add_videos_is_members_only(&conn);
        migrate_timestamps_to_unix(&conn);
        add_videos_details_checked_at(&conn);

        // Verify: auth table gone, users table exists with master role
        assert!(
            conn.prepare("SELECT 1 FROM auth").is_err(),
            "auth table should be dropped"
        );
        let (role, email): (String, String) = conn
            .query_row("SELECT role, email FROM users WHERE id = 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
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
        assert_eq!(
            show_live, 1,
            "show_livestreams should migrate from channels"
        );

        // Verify: hidden video migrated to user_videos
        let hidden_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM user_videos WHERE user_id = 1 AND is_hidden = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hidden_count, 1, "Hidden video should be in user_videos");

        // Verify: channels table no longer has is_favorite/show_livestreams
        assert!(
            conn.prepare("SELECT is_favorite FROM channels").is_err(),
            "channels should not have is_favorite"
        );
        assert!(
            conn.prepare("SELECT show_livestreams FROM channels")
                .is_err(),
            "channels should not have show_livestreams"
        );

        // Verify: videos table no longer has is_hidden
        assert!(
            conn.prepare("SELECT is_hidden FROM videos").is_err(),
            "videos should not have is_hidden"
        );

        // sessions テーブルは migrate() によって auth_id → user_id 列名変換されるが、
        // create_tables では sessions テーブルは定義されない（Cloudflare Access 移行済）。
        // legacy DB では migrate() 実行後に sessions テーブルが残存するが、
        // 新規 DB では sessions テーブルは存在しない（デッドコードとして整理済）。

        // Verify: groups have user_id
        let group_user: i64 = conn
            .query_row("SELECT user_id FROM groups WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(group_user, 1);

        for (table, column) in [
            ("users", "created_at"),
            ("channels", "created_at"),
            ("videos", "published_at"),
            ("groups", "created_at"),
            ("sessions", "expires_at"),
        ] {
            assert_eq!(
                super::column_declaration(&conn, table, column).map(|(kind, _)| kind),
                Some("INTEGER".to_string()),
                "{table}.{column} must use INTEGER affinity"
            );
        }
        let session_expiry: i64 = conn
            .query_row("SELECT expires_at FROM sessions WHERE id='s1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(session_expiry, 1767225600);
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

    // --- OAuth token column drop migration tests ---

    #[test]
    fn drop_users_oauth_token_columns_removes_legacy_columns_when_present() {
        // Simulate a legacy users table that still has the three OAuth columns.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                google_id TEXT,
                email TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'member',
                rss_token TEXT,
                access_token TEXT,
                refresh_token TEXT,
                token_expires_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT
            );",
        )
        .unwrap();

        assert!(
            super::column_exists(&conn, "users", "access_token"),
            "precondition: access_token must be present"
        );
        assert!(
            super::column_exists(&conn, "users", "refresh_token"),
            "precondition: refresh_token must be present"
        );
        assert!(
            super::column_exists(&conn, "users", "token_expires_at"),
            "precondition: token_expires_at must be present"
        );

        super::drop_users_oauth_token_columns(&conn);

        assert!(
            !super::column_exists(&conn, "users", "access_token"),
            "access_token should have been dropped"
        );
        assert!(
            !super::column_exists(&conn, "users", "refresh_token"),
            "refresh_token should have been dropped"
        );
        assert!(
            !super::column_exists(&conn, "users", "token_expires_at"),
            "token_expires_at should have been dropped"
        );
    }

    #[test]
    fn drop_users_oauth_token_columns_is_idempotent_when_already_absent() {
        // Fresh DB (from open_memory) does not have the OAuth columns.
        // Calling drop_users_oauth_token_columns must be a no-op without error.
        let conn = open_memory();
        assert!(
            !super::column_exists(&conn, "users", "access_token"),
            "precondition: access_token must not exist in fresh schema"
        );

        // Must not panic
        super::drop_users_oauth_token_columns(&conn);

        assert!(!super::column_exists(&conn, "users", "access_token"));
    }

    #[test]
    fn timestamp_migration_converts_absolute_text_and_nulls_ambiguous_text() {
        let conn = Connection::open_in_memory().unwrap();
        super::create_tables(&conn);
        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;
             DROP INDEX idx_videos_published;
             DROP INDEX idx_videos_channel;
             DROP TABLE videos;
             CREATE TABLE videos (
                id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, title TEXT NOT NULL,
                published_at TEXT, duration TEXT, is_short INTEGER NOT NULL DEFAULT 0,
                is_livestream INTEGER NOT NULL DEFAULT 0, is_members_only INTEGER NOT NULL DEFAULT 0,
                livestream_ended_at TEXT, fetched_at TEXT);
             INSERT INTO channels (id,title,created_at) VALUES ('UC1','Ch','2024-01-01T00:00:00Z');
             INSERT INTO users (email,created_at) VALUES ('shorts@example.com','2024-01-01T00:00:00Z');
             INSERT INTO user_channels (user_id,channel_id,hide_shorts,created_at)
             VALUES (1,'UC1',1,'2024-01-01T00:00:00Z');
             INSERT INTO videos (id,channel_id,title,published_at)
             VALUES ('absolute','UC1','A','2024-01-15T19:00:00+09:00'),
                    ('naive','UC1','N','2024-01-15 19:00:00'),
                    ('invalid','UC1','I','not-a-date');
             PRAGMA foreign_keys = ON;",
        )
        .unwrap();

        super::migrate_timestamps_to_unix(&conn);

        let kind: String = conn
            .query_row(
                "SELECT type FROM pragma_table_info('videos') WHERE name='published_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "INTEGER");
        let absolute: Option<i64> = conn
            .query_row(
                "SELECT published_at FROM videos WHERE id='absolute'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(absolute, Some(1705312800));
        let (hide_shorts, subscription_created_at): (i64, Option<i64>) = conn
            .query_row(
                "SELECT hide_shorts, created_at FROM user_channels
                 WHERE user_id=1 AND channel_id='UC1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(hide_shorts, 1);
        assert_eq!(subscription_created_at, Some(1704067200));
        for id in ["naive", "invalid"] {
            let value: Option<i64> = conn
                .query_row("SELECT published_at FROM videos WHERE id=?1", [id], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(value, None, "{id} must not be guessed");
        }
    }

    #[test]
    fn integer_schema_normalizes_text_storage_across_every_timestamp_table() {
        let conn = open_memory();
        conn.execute_batch(
            "INSERT INTO users (email,created_at,updated_at) VALUES ('a@example.com','2024-01-01T00:00:00Z','bad');
             INSERT INTO channels (id,title,last_fetched_at,created_at) VALUES ('UC1','Ch','2024-01-01T09:00:00+09:00','2024-01-01 00:00:00');
             INSERT INTO videos (id,channel_id,title,published_at,livestream_ended_at,fetched_at)
             VALUES ('v1','UC1','V','2024-01-01T00:00:00Z','bad','2024-01-01T00:00:00+00:00');
             INSERT INTO user_channels (user_id,channel_id,created_at) VALUES (1,'UC1','2024-01-01T00:00:00Z');
             INSERT INTO user_videos (user_id,video_id,created_at) VALUES (1,'v1','bad');
             INSERT INTO groups (user_id,name,created_at) VALUES (1,'G','2024-01-01T00:00:00Z');
             INSERT INTO channel_subscriptions (channel_id,hub_secret,subscribed_at,expires_at)
             VALUES ('UC1','s','2024-01-01T00:00:00Z','bad');",
        )
        .unwrap();

        migrate_timestamps_to_unix(&conn);

        for (table, column, expected_type) in [
            ("users", "created_at", "integer"),
            ("users", "updated_at", "null"),
            ("channels", "last_fetched_at", "integer"),
            ("channels", "created_at", "null"),
            ("videos", "published_at", "integer"),
            ("videos", "livestream_ended_at", "null"),
            ("videos", "fetched_at", "integer"),
            ("user_channels", "created_at", "integer"),
            ("user_videos", "created_at", "null"),
            ("groups", "created_at", "integer"),
            ("channel_subscriptions", "subscribed_at", "integer"),
            ("channel_subscriptions", "expires_at", "null"),
        ] {
            let actual: String = conn
                .query_row(
                    &format!("SELECT typeof({column}) FROM {table} LIMIT 1"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(actual, expected_type, "{table}.{column}");
        }
    }
}
