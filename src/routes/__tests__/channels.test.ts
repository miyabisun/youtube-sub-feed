import { describe, test, expect } from 'bun:test'
import { Hono } from 'hono'
import { Database } from 'bun:sqlite'

function setupApp() {
  const sqlite = new Database(':memory:')
  sqlite.exec('PRAGMA foreign_keys = ON')
  sqlite.exec(`CREATE TABLE channels (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, thumbnail_url TEXT,
    upload_playlist_id TEXT, show_livestreams INTEGER NOT NULL DEFAULT 0,
    last_fetched_at TEXT, created_at TEXT NOT NULL
  )`)
  sqlite.exec(`CREATE TABLE videos (
    id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, title TEXT NOT NULL,
    thumbnail_url TEXT, published_at TEXT, duration TEXT, is_short INTEGER NOT NULL DEFAULT 0,
    is_livestream INTEGER NOT NULL DEFAULT 0, livestream_ended_at TEXT,
    is_hidden INTEGER NOT NULL DEFAULT 0, fetched_at TEXT,
    FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
  )`)
  sqlite.exec(`CREATE TABLE groups (
    id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL
  )`)
  sqlite.exec(`CREATE TABLE channel_groups (
    channel_id TEXT NOT NULL, group_id INTEGER NOT NULL,
    PRIMARY KEY (channel_id, group_id),
    FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
    FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
  )`)

  sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Alpha Channel', '2024-01-01')")
  sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC2', 'Beta Channel', '2024-01-01')")
  sqlite.exec("INSERT INTO videos (id, channel_id, title, published_at) VALUES ('v1', 'UC1', 'V1', '2024-06-01')")
  sqlite.exec("INSERT INTO videos (id, channel_id, title, published_at, is_hidden) VALUES ('v2', 'UC1', 'V2 Hidden', '2024-06-02', 1)")

  const app = new Hono()

  app.get('/api/channels', (c) => {
    const rows = sqlite.query(`SELECT c.id, c.title, c.show_livestreams FROM channels c ORDER BY c.title COLLATE NOCASE`).all()
    return c.json(rows)
  })

  app.get('/api/channels/:id/videos', (c) => {
    const id = c.req.param('id')
    const limit = Math.min(Number(c.req.query('limit')) || 100, 500)
    const offset = Number(c.req.query('offset')) || 0
    const videos = sqlite.query('SELECT id, title, is_hidden FROM videos WHERE channel_id = ? ORDER BY published_at DESC LIMIT ? OFFSET ?').all(id, limit, offset)
    return c.json(videos)
  })

  app.patch('/api/channels/:id', async (c) => {
    const id = c.req.param('id')
    const body = await c.req.json() as any
    if (body.show_livestreams === undefined) return c.json({ error: 'No fields to update' }, 400)
    const val = Number(body.show_livestreams)
    if (val !== 0 && val !== 1) return c.json({ error: 'show_livestreams must be 0 or 1' }, 400)
    sqlite.query('UPDATE channels SET show_livestreams = ? WHERE id = ?').run(val, id)
    return c.json({ ok: true })
  })

  return { app, sqlite }
}

describe('GET /api/channels', () => {
  test('returns all channels sorted by title', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/channels')
    const body: any[] = await res.json()
    expect(body).toHaveLength(2)
    expect(body[0].title).toBe('Alpha Channel')
    expect(body[1].title).toBe('Beta Channel')
  })
})

describe('GET /api/channels/:id/videos', () => {
  test('returns all videos including hidden', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/channels/UC1/videos')
    const body: any[] = await res.json()
    expect(body).toHaveLength(2)
    expect(body.some((v: any) => v.is_hidden === 1)).toBe(true)
  })

  test('supports pagination', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/channels/UC1/videos?limit=1')
    const body: any[] = await res.json()
    expect(body).toHaveLength(1)
  })
})

describe('PATCH /api/channels/:id', () => {
  test('updates show_livestreams setting', async () => {
    const { app, sqlite } = setupApp()
    const res = await app.request('/api/channels/UC1', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ show_livestreams: 1 }),
    })
    expect(res.status).toBe(200)

    const ch = sqlite.query("SELECT show_livestreams FROM channels WHERE id = 'UC1'").get() as any
    expect(ch.show_livestreams).toBe(1)
  })

  test('returns 400 for invalid show_livestreams value', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/channels/UC1', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ show_livestreams: 2 }),
    })
    expect(res.status).toBe(400)
  })

  test('returns 400 for empty update', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/channels/UC1', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    })
    expect(res.status).toBe(400)
  })
})
