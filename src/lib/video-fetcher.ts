import { sqlite } from '../db/index.js'
import { fetchPlaylistItems, fetchVideoDetails, fetchUUSHPlaylist } from './youtube-api.js'
import { isShortDuration } from './duration.js'
import { notifyNewVideo } from './discord.js'
import cache from './cache.js'
import type { Database } from 'bun:sqlite'

interface FetchDeps {
  db: Database
  fetchPlaylistItems: typeof fetchPlaylistItems
  fetchVideoDetails: typeof fetchVideoDetails
  fetchUUSHPlaylist: typeof fetchUUSHPlaylist
  notifyNewVideo: typeof notifyNewVideo
  cache: typeof cache
}

const defaultDeps: FetchDeps = {
  db: sqlite,
  fetchPlaylistItems,
  fetchVideoDetails,
  fetchUUSHPlaylist,
  notifyNewVideo,
  cache,
}

export async function fetchChannelVideosCore(
  channelId: string,
  accessToken: string,
  { notify = true } = {},
  deps: FetchDeps = defaultDeps,
): Promise<string[]> {
  const { db, cache: c } = deps

  const channel = db.query('SELECT upload_playlist_id FROM channels WHERE id = ?').get(channelId) as { upload_playlist_id: string } | null
  if (!channel) return []

  // Fetch latest videos from UU playlist
  let items: Awaited<ReturnType<typeof deps.fetchPlaylistItems>>
  try {
    items = await deps.fetchPlaylistItems(channel.upload_playlist_id, accessToken, 10)
  } catch (e: any) {
    if (e?.status === 404 && e?.reason === 'playlistNotFound') {
      console.log(`[video-fetcher] Playlist not found for ${channelId}, deleting channel`)
      db.query('DELETE FROM channels WHERE id = ?').run(channelId)
      return []
    }
    throw e
  }

  if (items.length === 0) return []

  const now = new Date().toISOString()
  const newVideoIds: string[] = []

  // UPSERT playlist items (update title/thumbnail for existing, insert new)
  const upsertStmt = db.query(`
    INSERT INTO videos (id, channel_id, title, thumbnail_url, published_at, fetched_at)
    VALUES (?, ?, ?, ?, ?, ?)
    ON CONFLICT(id) DO UPDATE SET
      title = excluded.title,
      thumbnail_url = excluded.thumbnail_url
    WHERE title != excluded.title OR thumbnail_url != excluded.thumbnail_url
  `)

  // Batch check for existing videos
  const videoIds = items.map((item) => item.videoId)
  const placeholders = videoIds.map(() => '?').join(',')
  const existingRows = db.query(`SELECT id FROM videos WHERE id IN (${placeholders})`).all(...videoIds) as { id: string }[]
  const existingIds = new Set(existingRows.map((r) => r.id))

  for (const item of items) {
    upsertStmt.run(item.videoId, channelId, item.title, item.thumbnailUrl, item.publishedAt, now)
    if (!existingIds.has(item.videoId)) {
      newVideoIds.push(item.videoId)
    }
  }

  if (newVideoIds.length === 0) {
    // Update last_fetched_at even when no new videos
    db.query('UPDATE channels SET last_fetched_at = ? WHERE id = ?').run(now, channelId)
    return []
  }

  // Fetch details for new videos only (duration, livestream info)
  const details = await deps.fetchVideoDetails(newVideoIds, accessToken)

  const updateStmt = db.query('UPDATE videos SET duration = ?, is_livestream = ?, livestream_ended_at = ?, is_short = ? WHERE id = ?')

  for (const detail of details) {
    let isShort = 0
    if (isShortDuration(detail.duration)) {
      // Check UUSH playlist for short confirmation
      const cacheKey = `uush:${channelId}`
      let uushIds = c.get(cacheKey) as string[] | null
      if (uushIds === null) {
        uushIds = await deps.fetchUUSHPlaylist(channelId, accessToken)
        c.set(cacheKey, uushIds, 3600) // cache for 1 hour
      }
      isShort = uushIds.includes(detail.id) ? 1 : 0
    }

    updateStmt.run(
      detail.duration,
      detail.isLivestream ? 1 : 0,
      detail.livestreamEndedAt,
      isShort,
      detail.id
    )
  }

  db.query('UPDATE channels SET last_fetched_at = ? WHERE id = ?').run(now, channelId)

  // Notify Discord for each new video (skip during initial setup)
  if (notify) {
    const channelRow = db.query('SELECT title FROM channels WHERE id = ?').get(channelId) as { title: string } | null
    for (const videoId of newVideoIds) {
      const video = db.query('SELECT id, title, thumbnail_url, published_at, is_short FROM videos WHERE id = ?').get(videoId) as any
      if (video && channelRow) {
        deps.notifyNewVideo({ ...video, channel_title: channelRow.title }).catch(() => {})
      }
    }
  }

  return newVideoIds
}

export async function fetchChannelVideos(channelId: string, accessToken: string, opts: { notify?: boolean } = {}): Promise<string[]> {
  return fetchChannelVideosCore(channelId, accessToken, opts)
}
