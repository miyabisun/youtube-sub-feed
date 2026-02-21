import { describe, test, expect } from 'bun:test'
import { Hono } from 'hono'
import { Database } from 'bun:sqlite'

function setupApp() {
  const sqlite = new Database(':memory:')
  sqlite.exec('PRAGMA foreign_keys = ON')
  sqlite.exec(`CREATE TABLE channels (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, thumbnail_url TEXT,
    upload_playlist_id TEXT, show_livestreams INTEGER NOT NULL DEFAULT 0,
    fast_polling INTEGER NOT NULL DEFAULT 0, last_fetched_at TEXT, created_at TEXT NOT NULL
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

  // Seed data
  sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Channel 1', '2024-01-01')")
  sqlite.exec("INSERT INTO channels (id, title, show_livestreams, created_at) VALUES ('UC2', 'Channel 2', 1, '2024-01-01')")
  sqlite.exec("INSERT INTO videos (id, channel_id, title, published_at, is_hidden) VALUES ('v1', 'UC1', 'Video 1', '2024-06-01', 0)")
  sqlite.exec("INSERT INTO videos (id, channel_id, title, published_at, is_hidden) VALUES ('v2', 'UC1', 'Hidden Video', '2024-06-02', 1)")
  sqlite.exec("INSERT INTO videos (id, channel_id, title, published_at, is_livestream) VALUES ('v3', 'UC1', 'Livestream', '2024-06-03', 1)")
  sqlite.exec("INSERT INTO videos (id, channel_id, title, published_at, is_livestream) VALUES ('v4', 'UC2', 'Allowed Live', '2024-06-04', 1)")
  sqlite.exec("INSERT INTO videos (id, channel_id, title, published_at) VALUES ('v5', 'UC2', 'Normal V', '2024-06-05')")

  sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('Group A', 0, '2024-01-01')")
  sqlite.exec("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)")

  const app = new Hono()

  app.get('/api/feed', (c) => {
    const limit = Math.min(Number(c.req.query('limit')) || 100, 500)
    const offset = Number(c.req.query('offset')) || 0
    const groupId = c.req.query('group')

    let query: string
    let params: any[]

    if (groupId) {
      query = `SELECT v.id, v.title, v.is_hidden, v.is_livestream, c.title as channel_title
        FROM videos v JOIN channels c ON v.channel_id = c.id
        JOIN channel_groups cg ON v.channel_id = cg.channel_id
        WHERE v.is_hidden = 0 AND (v.is_livestream = 0 OR c.show_livestreams = 1) AND cg.group_id = ?
        ORDER BY v.published_at DESC LIMIT ? OFFSET ?`
      params = [Number(groupId), limit, offset]
    } else {
      query = `SELECT v.id, v.title, v.is_hidden, v.is_livestream, c.title as channel_title
        FROM videos v JOIN channels c ON v.channel_id = c.id
        WHERE v.is_hidden = 0 AND (v.is_livestream = 0 OR c.show_livestreams = 1)
        ORDER BY v.published_at DESC LIMIT ? OFFSET ?`
      params = [limit, offset]
    }
    return c.json(sqlite.query(query).all(...params))
  })

  app.patch('/api/videos/:id/hide', (c) => {
    sqlite.query('UPDATE videos SET is_hidden = 1 WHERE id = ?').run(c.req.param('id'))
    return c.json({ ok: true })
  })

  app.patch('/api/videos/:id/unhide', (c) => {
    sqlite.query('UPDATE videos SET is_hidden = 0 WHERE id = ?').run(c.req.param('id'))
    return c.json({ ok: true })
  })

  return { app, sqlite }
}

describe('GET /api/feed', () => {
  test('excludes hidden videos', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/feed')
    const body: any[] = await res.json()
    expect(body.every((v: any) => v.id !== 'v2')).toBe(true)
  })

  test('excludes livestreams from channels without show_livestreams', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/feed')
    const body: any[] = await res.json()
    // v3 is livestream on UC1 (show_livestreams=0) - should be excluded
    expect(body.every((v: any) => v.id !== 'v3')).toBe(true)
    // v4 is livestream on UC2 (show_livestreams=1) - should be included
    expect(body.some((v: any) => v.id === 'v4')).toBe(true)
  })

  test('filters by group', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/feed?group=1')
    const body: any[] = await res.json()
    // Only UC1 is in group 1
    expect(body.every((v: any) => v.channel_title === 'Channel 1')).toBe(true)
    expect(body.some((v: any) => v.id === 'v1')).toBe(true)
  })

  test('paginates results', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/feed?limit=1&offset=0')
    const body: any[] = await res.json()
    expect(body).toHaveLength(1)

    const res2 = await app.request('/api/feed?limit=1&offset=1')
    const body2: any[] = await res2.json()
    expect(body2).toHaveLength(1)
    expect(body2[0].id).not.toBe(body[0].id)
  })
})

describe('PATCH /api/videos/:id/hide', () => {
  test('hides a video', async () => {
    const { app, sqlite } = setupApp()
    const res = await app.request('/api/videos/v1/hide', { method: 'PATCH' })
    expect(res.status).toBe(200)

    const video = sqlite.query("SELECT is_hidden FROM videos WHERE id = 'v1'").get() as any
    expect(video.is_hidden).toBe(1)
  })
})

describe('PATCH /api/videos/:id/unhide', () => {
  test('unhides a video', async () => {
    const { app, sqlite } = setupApp()
    const res = await app.request('/api/videos/v2/unhide', { method: 'PATCH' })
    expect(res.status).toBe(200)

    const video = sqlite.query("SELECT is_hidden FROM videos WHERE id = 'v2'").get() as any
    expect(video.is_hidden).toBe(0)
  })
})
