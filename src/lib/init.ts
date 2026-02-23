import { sqlite } from '../db/index.js'

export default function init() {
  sqlite.exec(`
    CREATE TABLE IF NOT EXISTS channels (
      id TEXT PRIMARY KEY,
      title TEXT NOT NULL,
      thumbnail_url TEXT,
      upload_playlist_id TEXT,
      show_livestreams INTEGER NOT NULL DEFAULT 0,
      last_fetched_at TEXT,
      created_at TEXT NOT NULL
    )
  `)

  sqlite.exec(`
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
    )
  `)

  sqlite.exec(`
    CREATE TABLE IF NOT EXISTS groups (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL,
      sort_order INTEGER NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL
    )
  `)

  sqlite.exec(`
    CREATE TABLE IF NOT EXISTS channel_groups (
      channel_id TEXT NOT NULL,
      group_id INTEGER NOT NULL,
      PRIMARY KEY (channel_id, group_id),
      FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
      FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
    )
  `)

  sqlite.exec(`
    CREATE TABLE IF NOT EXISTS auth (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      google_id TEXT NOT NULL,
      email TEXT NOT NULL,
      access_token TEXT,
      refresh_token TEXT,
      token_expires_at TEXT,
      updated_at TEXT
    )
  `)

  sqlite.exec(`
    CREATE TABLE IF NOT EXISTS sessions (
      id TEXT PRIMARY KEY,
      auth_id INTEGER NOT NULL,
      expires_at TEXT NOT NULL,
      created_at TEXT NOT NULL,
      FOREIGN KEY (auth_id) REFERENCES auth(id) ON DELETE CASCADE
    )
  `)

  // Indexes
  sqlite.exec('CREATE INDEX IF NOT EXISTS idx_videos_published ON videos (published_at DESC)')
  sqlite.exec('CREATE INDEX IF NOT EXISTS idx_videos_channel ON videos (channel_id)')
  sqlite.exec('CREATE INDEX IF NOT EXISTS idx_videos_hidden ON videos (is_hidden, published_at DESC)')
  sqlite.exec('CREATE UNIQUE INDEX IF NOT EXISTS idx_auth_google_id ON auth(google_id)')
}
