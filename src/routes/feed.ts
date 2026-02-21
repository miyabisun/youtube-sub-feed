import { Hono } from 'hono'
import { sqlite } from '../db/index.js'

const feed = new Hono()

feed.get('/api/feed', (c) => {
  const limit = Math.min(Number(c.req.query('limit')) || 100, 500)
  const offset = Number(c.req.query('offset')) || 0
  const groupId = c.req.query('group')

  let query: string
  let params: any[]

  if (groupId) {
    query = `
      SELECT v.id, v.channel_id, v.title, v.thumbnail_url, v.published_at,
             v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
             c.title as channel_title, c.thumbnail_url as channel_thumbnail
      FROM videos v
      JOIN channels c ON v.channel_id = c.id
      JOIN channel_groups cg ON v.channel_id = cg.channel_id
      WHERE v.is_hidden = 0
        AND (v.is_livestream = 0 OR c.show_livestreams = 1)
        AND cg.group_id = ?
      ORDER BY v.published_at DESC
      LIMIT ? OFFSET ?
    `
    params = [Number(groupId), limit, offset]
  } else {
    query = `
      SELECT v.id, v.channel_id, v.title, v.thumbnail_url, v.published_at,
             v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
             c.title as channel_title, c.thumbnail_url as channel_thumbnail
      FROM videos v
      JOIN channels c ON v.channel_id = c.id
      WHERE v.is_hidden = 0
        AND (v.is_livestream = 0 OR c.show_livestreams = 1)
      ORDER BY v.published_at DESC
      LIMIT ? OFFSET ?
    `
    params = [limit, offset]
  }

  const videos = sqlite.query(query).all(...params)
  return c.json(videos)
})

feed.patch('/api/videos/:id/hide', (c) => {
  const id = c.req.param('id')
  sqlite.query('UPDATE videos SET is_hidden = 1 WHERE id = ?').run(id)
  return c.json({ ok: true })
})

feed.patch('/api/videos/:id/unhide', (c) => {
  const id = c.req.param('id')
  sqlite.query('UPDATE videos SET is_hidden = 0 WHERE id = ?').run(id)
  return c.json({ ok: true })
})

export default feed
