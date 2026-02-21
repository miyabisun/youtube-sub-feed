import { describe, test, expect } from 'bun:test'
import { Database } from 'bun:sqlite'

function setupAndInit() {
  const sqlite = new Database(':memory:')
  sqlite.exec('PRAGMA journal_mode = WAL')
  sqlite.exec('PRAGMA foreign_keys = ON')
  runInit(sqlite)
  return sqlite
}

function runInit(sqlite: Database) {
  sqlite.exec(`
    CREATE TABLE IF NOT EXISTS channels (
      id TEXT PRIMARY KEY,
      title TEXT NOT NULL,
      thumbnail_url TEXT,
      upload_playlist_id TEXT,
      show_livestreams INTEGER NOT NULL DEFAULT 0,
      fast_polling INTEGER NOT NULL DEFAULT 0,
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
  sqlite.exec('CREATE INDEX IF NOT EXISTS idx_videos_published ON videos (published_at DESC)')
  sqlite.exec('CREATE INDEX IF NOT EXISTS idx_videos_channel ON videos (channel_id)')
  sqlite.exec('CREATE INDEX IF NOT EXISTS idx_videos_hidden ON videos (is_hidden, published_at DESC)')
  sqlite.exec('CREATE UNIQUE INDEX IF NOT EXISTS idx_auth_google_id ON auth(google_id)')
}

function getTableNames(sqlite: Database): string[] {
  return sqlite.query("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all().map((r: any) => r.name)
}

function getIndexNames(sqlite: Database): string[] {
  return sqlite.query("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name").all().map((r: any) => r.name)
}

describe('init', () => {
  test('creates all 6 tables', () => {
    const sqlite = setupAndInit()
    const tables = getTableNames(sqlite)
    expect(tables).toContain('channels')
    expect(tables).toContain('videos')
    expect(tables).toContain('groups')
    expect(tables).toContain('channel_groups')
    expect(tables).toContain('auth')
    expect(tables).toContain('sessions')
  })

  test('creates all indexes', () => {
    const sqlite = setupAndInit()
    const indexes = getIndexNames(sqlite)
    expect(indexes).toContain('idx_videos_published')
    expect(indexes).toContain('idx_videos_channel')
    expect(indexes).toContain('idx_videos_hidden')
    expect(indexes).toContain('idx_auth_google_id')
  })

  test('auth.google_id is unique', () => {
    const sqlite = setupAndInit()
    sqlite.exec("INSERT INTO auth (google_id, email) VALUES ('g1', 'a@b.com')")
    expect(() => {
      sqlite.exec("INSERT INTO auth (google_id, email) VALUES ('g1', 'c@d.com')")
    }).toThrow()
  })

  test('is idempotent - running twice does not error', () => {
    const sqlite = setupAndInit()
    runInit(sqlite)
    const tables = getTableNames(sqlite)
    expect(tables).toContain('channels')
    expect(tables).toContain('videos')
  })

  test('videos.channel_id cascades on channel delete', () => {
    const sqlite = setupAndInit()
    sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC123', 'Test', '2024-01-01')")
    sqlite.exec("INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC123', 'Video 1')")
    sqlite.exec("DELETE FROM channels WHERE id = 'UC123'")
    const videos = sqlite.query("SELECT * FROM videos WHERE channel_id = 'UC123'").all()
    expect(videos).toHaveLength(0)
  })

  test('channel_groups cascades on channel delete', () => {
    const sqlite = setupAndInit()
    sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC123', 'Test', '2024-01-01')")
    sqlite.exec("INSERT INTO groups (name, created_at) VALUES ('Group1', '2024-01-01')")
    sqlite.exec("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC123', 1)")
    sqlite.exec("DELETE FROM channels WHERE id = 'UC123'")
    const cg = sqlite.query("SELECT * FROM channel_groups WHERE channel_id = 'UC123'").all()
    expect(cg).toHaveLength(0)
  })

  test('channel_groups cascades on group delete', () => {
    const sqlite = setupAndInit()
    sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC123', 'Test', '2024-01-01')")
    sqlite.exec("INSERT INTO groups (name, created_at) VALUES ('Group1', '2024-01-01')")
    sqlite.exec("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC123', 1)")
    sqlite.exec("DELETE FROM groups WHERE id = 1")
    const cg = sqlite.query("SELECT * FROM channel_groups WHERE group_id = 1").all()
    expect(cg).toHaveLength(0)
  })

  test('sessions cascades on auth delete', () => {
    const sqlite = setupAndInit()
    sqlite.exec("INSERT INTO auth (google_id, email) VALUES ('g123', 'test@example.com')")
    sqlite.exec("INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES ('sess1', 1, '2025-01-01', '2024-01-01')")
    sqlite.exec("DELETE FROM auth WHERE id = 1")
    const sessions = sqlite.query("SELECT * FROM sessions WHERE auth_id = 1").all()
    expect(sessions).toHaveLength(0)
  })

  test('videos table has correct default values', () => {
    const sqlite = setupAndInit()
    sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC123', 'Test', '2024-01-01')")
    sqlite.exec("INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC123', 'Video 1')")
    const video: any = sqlite.query("SELECT * FROM videos WHERE id = 'v1'").get()
    expect(video.is_short).toBe(0)
    expect(video.is_livestream).toBe(0)
    expect(video.is_hidden).toBe(0)
  })

  test('channels table has correct default values', () => {
    const sqlite = setupAndInit()
    sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC123', 'Test', '2024-01-01')")
    const channel: any = sqlite.query("SELECT * FROM channels WHERE id = 'UC123'").get()
    expect(channel.show_livestreams).toBe(0)
    expect(channel.fast_polling).toBe(0)
  })
})
