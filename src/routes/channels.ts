import { Hono } from 'hono'
import { sqlite } from '../db/index.js'
import { getValidAccessToken } from '../lib/token-manager.js'
import { syncSubscriptions } from '../lib/channel-sync.js'
import { fetchChannelVideos } from '../lib/video-fetcher.js'

const channels = new Hono()

channels.get('/api/channels', (c) => {
  const rows = sqlite.query(`
    SELECT c.id, c.title, c.thumbnail_url, c.show_livestreams, c.fast_polling, c.last_fetched_at,
      (SELECT GROUP_CONCAT(g.name, ', ')
       FROM channel_groups cg JOIN groups g ON cg.group_id = g.id
       WHERE cg.channel_id = c.id) as group_names
    FROM channels c
    ORDER BY c.title COLLATE NOCASE
  `).all()
  return c.json(rows)
})

channels.get('/api/channels/:id/videos', (c) => {
  const id = c.req.param('id')
  const limit = Math.min(Number(c.req.query('limit')) || 100, 500)
  const offset = Number(c.req.query('offset')) || 0

  const videos = sqlite.query(`
    SELECT id, title, thumbnail_url, published_at, duration,
           is_short, is_livestream, livestream_ended_at, is_hidden
    FROM videos
    WHERE channel_id = ?
    ORDER BY published_at DESC
    LIMIT ? OFFSET ?
  `).all(id, limit, offset)
  return c.json(videos)
})

channels.post('/api/channels/sync', async (c) => {
  const token = await getValidAccessToken()
  if (!token) return c.json({ error: 'No valid token' }, 401)
  const result = await syncSubscriptions(token)
  return c.json(result)
})

channels.post('/api/channels/:id/refresh', async (c) => {
  const id = c.req.param('id')
  const token = await getValidAccessToken()
  if (!token) return c.json({ error: 'No valid token' }, 401)
  const newVideoIds = await fetchChannelVideos(id, token)
  return c.json({ newVideos: newVideoIds.length })
})

channels.patch('/api/channels/:id', async (c) => {
  const id = c.req.param('id')
  const body = await c.req.json<{ show_livestreams?: number; fast_polling?: number }>()

  const sets: string[] = []
  const params: any[] = []

  if (body.show_livestreams !== undefined) {
    const val = Number(body.show_livestreams)
    if (val !== 0 && val !== 1) return c.json({ error: 'show_livestreams must be 0 or 1' }, 400)
    sets.push('show_livestreams = ?')
    params.push(val)
  }
  if (body.fast_polling !== undefined) {
    const val = Number(body.fast_polling)
    if (val !== 0 && val !== 1) return c.json({ error: 'fast_polling must be 0 or 1' }, 400)
    sets.push('fast_polling = ?')
    params.push(val)
  }

  if (sets.length === 0) return c.json({ error: 'No fields to update' }, 400)

  params.push(id)
  sqlite.query(`UPDATE channels SET ${sets.join(', ')} WHERE id = ?`).run(...params)
  return c.json({ ok: true })
})

export default channels
