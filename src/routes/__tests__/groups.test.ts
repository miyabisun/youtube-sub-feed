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

  sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Ch1', '2024-01-01')")
  sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC2', 'Ch2', '2024-01-01')")

  const app = new Hono()

  app.get('/api/groups', (c) => {
    return c.json(sqlite.query('SELECT * FROM groups ORDER BY sort_order ASC, id ASC').all())
  })

  app.post('/api/groups', async (c) => {
    const { name } = await c.req.json() as any
    if (!name) return c.json({ error: 'Name is required' }, 400)
    const maxOrder = sqlite.query('SELECT MAX(sort_order) as max FROM groups').get() as any
    const sortOrder = (maxOrder?.max ?? -1) + 1
    sqlite.query('INSERT INTO groups (name, sort_order, created_at) VALUES (?, ?, ?)').run(name, sortOrder, new Date().toISOString())
    const row = sqlite.query('SELECT * FROM groups WHERE rowid = last_insert_rowid()').get()
    return c.json(row, 201)
  })

  app.patch('/api/groups/:id', async (c) => {
    const id = Number(c.req.param('id'))
    const { name } = await c.req.json() as any
    if (!name) return c.json({ error: 'Name is required' }, 400)
    sqlite.query('UPDATE groups SET name = ? WHERE id = ?').run(name, id)
    return c.json({ ok: true })
  })

  app.put('/api/groups/reorder', async (c) => {
    const { order } = await c.req.json() as any
    if (!Array.isArray(order)) return c.json({ error: 'order must be an array' }, 400)
    const stmt = sqlite.query('UPDATE groups SET sort_order = ? WHERE id = ?')
    for (let i = 0; i < order.length; i++) stmt.run(i, order[i])
    return c.json({ ok: true })
  })

  app.delete('/api/groups/:id', (c) => {
    sqlite.query('DELETE FROM groups WHERE id = ?').run(Number(c.req.param('id')))
    return c.json({ ok: true })
  })

  app.get('/api/groups/:id/channels', (c) => {
    const groupId = Number(c.req.param('id'))
    const rows = sqlite.query('SELECT channel_id FROM channel_groups WHERE group_id = ?').all(groupId) as { channel_id: string }[]
    return c.json(rows.map((r: any) => r.channel_id))
  })

  app.put('/api/groups/:id/channels', async (c) => {
    const groupId = Number(c.req.param('id'))
    const { channelIds } = await c.req.json() as any
    if (!Array.isArray(channelIds)) return c.json({ error: 'channelIds must be an array' }, 400)
    sqlite.query('DELETE FROM channel_groups WHERE group_id = ?').run(groupId)
    const ins = sqlite.query('INSERT INTO channel_groups (channel_id, group_id) VALUES (?, ?)')
    for (const cid of channelIds) ins.run(cid, groupId)
    return c.json({ ok: true })
  })

  return { app, sqlite }
}

describe('groups CRUD', () => {
  test('creates a group and lists it', async () => {
    const { app } = setupApp()
    const createRes = await app.request('/api/groups', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'Gaming' }),
    })
    expect(createRes.status).toBe(201)
    const created: any = await createRes.json()
    expect(created.name).toBe('Gaming')

    const listRes = await app.request('/api/groups')
    const list: any[] = await listRes.json()
    expect(list).toHaveLength(1)
    expect(list[0].name).toBe('Gaming')
  })

  test('updates group name', async () => {
    const { app, sqlite } = setupApp()
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('Old', 0, '2024-01-01')")

    const res = await app.request('/api/groups/1', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'New' }),
    })
    expect(res.status).toBe(200)

    const group = sqlite.query("SELECT name FROM groups WHERE id = 1").get() as any
    expect(group.name).toBe('New')
  })

  test('deletes a group', async () => {
    const { app, sqlite } = setupApp()
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('ToDelete', 0, '2024-01-01')")

    const res = await app.request('/api/groups/1', { method: 'DELETE' })
    expect(res.status).toBe(200)

    const groups = sqlite.query('SELECT * FROM groups').all()
    expect(groups).toHaveLength(0)
  })
})

describe('groups reorder', () => {
  test('reorders groups by id array', async () => {
    const { app, sqlite } = setupApp()
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('A', 0, '2024-01-01')")
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('B', 1, '2024-01-01')")
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('C', 2, '2024-01-01')")

    const res = await app.request('/api/groups/reorder', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ order: [3, 1, 2] }),
    })
    expect(res.status).toBe(200)

    const groups = sqlite.query('SELECT id, sort_order FROM groups ORDER BY sort_order').all() as any[]
    expect(groups[0].id).toBe(3)
    expect(groups[1].id).toBe(1)
    expect(groups[2].id).toBe(2)
  })
})

describe('group channel assignment', () => {
  test('assigns channels to a group', async () => {
    const { app, sqlite } = setupApp()
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 0, '2024-01-01')")

    const res = await app.request('/api/groups/1/channels', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ channelIds: ['UC1', 'UC2'] }),
    })
    expect(res.status).toBe(200)

    const cg = sqlite.query('SELECT channel_id FROM channel_groups WHERE group_id = 1 ORDER BY channel_id').all() as any[]
    expect(cg).toHaveLength(2)
    expect(cg[0].channel_id).toBe('UC1')
  })

  test('full replaces channel assignment', async () => {
    const { app, sqlite } = setupApp()
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 0, '2024-01-01')")
    sqlite.exec("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)")
    sqlite.exec("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC2', 1)")

    const res = await app.request('/api/groups/1/channels', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ channelIds: ['UC2'] }),
    })
    expect(res.status).toBe(200)

    const cg = sqlite.query('SELECT channel_id FROM channel_groups WHERE group_id = 1').all() as any[]
    expect(cg).toHaveLength(1)
    expect(cg[0].channel_id).toBe('UC2')
  })

  test('returns assigned channel IDs for a group', async () => {
    const { app, sqlite } = setupApp()
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 0, '2024-01-01')")
    sqlite.exec("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)")
    sqlite.exec("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC2', 1)")

    const res = await app.request('/api/groups/1/channels')
    expect(res.status).toBe(200)
    const ids: string[] = await res.json()
    expect(ids).toHaveLength(2)
    expect(ids).toContain('UC1')
    expect(ids).toContain('UC2')
  })

  test('returns empty array for group with no channels', async () => {
    const { app, sqlite } = setupApp()
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 0, '2024-01-01')")

    const res = await app.request('/api/groups/1/channels')
    expect(res.status).toBe(200)
    const ids: string[] = await res.json()
    expect(ids).toHaveLength(0)
  })

  test('cascade deletes channel_groups when group is deleted', async () => {
    const { app, sqlite } = setupApp()
    sqlite.exec("INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 0, '2024-01-01')")
    sqlite.exec("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)")

    await app.request('/api/groups/1', { method: 'DELETE' })
    const cg = sqlite.query('SELECT * FROM channel_groups WHERE group_id = 1').all()
    expect(cg).toHaveLength(0)
  })
})
