import { Hono } from 'hono'
import { sqlite } from '../db/index.js'

const groups = new Hono()

groups.get('/api/groups', (c) => {
  const rows = sqlite.query('SELECT * FROM groups ORDER BY sort_order ASC, id ASC').all()
  return c.json(rows)
})

groups.post('/api/groups', async (c) => {
  const { name } = await c.req.json<{ name: string }>()
  if (!name) return c.json({ error: 'Name is required' }, 400)
  if (name.length > 50) return c.json({ error: 'Name must be 50 characters or less' }, 400)

  const maxOrder = sqlite.query('SELECT MAX(sort_order) as max FROM groups').get() as { max: number | null }
  const sortOrder = (maxOrder?.max ?? -1) + 1
  const now = new Date().toISOString()

  sqlite.query('INSERT INTO groups (name, sort_order, created_at) VALUES (?, ?, ?)').run(name, sortOrder, now)
  const row = sqlite.query('SELECT * FROM groups WHERE rowid = last_insert_rowid()').get()
  return c.json(row, 201)
})

groups.patch('/api/groups/:id', async (c) => {
  const id = Number(c.req.param('id'))
  const { name } = await c.req.json<{ name: string }>()
  if (!name) return c.json({ error: 'Name is required' }, 400)
  if (name.length > 50) return c.json({ error: 'Name must be 50 characters or less' }, 400)

  sqlite.query('UPDATE groups SET name = ? WHERE id = ?').run(name, id)
  return c.json({ ok: true })
})

groups.put('/api/groups/reorder', async (c) => {
  const { order } = await c.req.json<{ order: number[] }>()
  if (!Array.isArray(order)) return c.json({ error: 'order must be an array' }, 400)

  const updateStmt = sqlite.query('UPDATE groups SET sort_order = ? WHERE id = ?')
  for (let i = 0; i < order.length; i++) {
    updateStmt.run(i, order[i])
  }
  return c.json({ ok: true })
})

groups.delete('/api/groups/:id', (c) => {
  const id = Number(c.req.param('id'))
  sqlite.query('DELETE FROM groups WHERE id = ?').run(id)
  return c.json({ ok: true })
})

groups.get('/api/groups/:id/channels', (c) => {
  const groupId = Number(c.req.param('id'))
  const rows = sqlite.query('SELECT channel_id FROM channel_groups WHERE group_id = ?').all(groupId) as { channel_id: string }[]
  return c.json(rows.map((r) => r.channel_id))
})

groups.put('/api/groups/:id/channels', async (c) => {
  const groupId = Number(c.req.param('id'))
  const { channelIds } = await c.req.json<{ channelIds: string[] }>()
  if (!Array.isArray(channelIds)) return c.json({ error: 'channelIds must be an array' }, 400)

  // Full replace
  sqlite.query('DELETE FROM channel_groups WHERE group_id = ?').run(groupId)
  const insertStmt = sqlite.query('INSERT INTO channel_groups (channel_id, group_id) VALUES (?, ?)')
  for (const channelId of channelIds) {
    insertStmt.run(channelId, groupId)
  }
  return c.json({ ok: true })
})

export default groups
