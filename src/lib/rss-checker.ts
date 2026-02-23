import { sqlite } from '../db/index.js'
import { fetchRssFeed } from './youtube-rss.js'
import type { Database } from 'bun:sqlite'

interface RssCheckResult {
  hasNewVideos: boolean
  newVideoIds: string[]
}

interface RssCheckerDeps {
  db: Database
  fetchRssFeed: (channelId: string) => Promise<{ videoId: string }[]>
}

const defaultDeps: RssCheckerDeps = {
  db: sqlite,
  fetchRssFeed,
}

export async function checkRssForNewVideosCore(
  channelId: string,
  deps: RssCheckerDeps = defaultDeps,
): Promise<RssCheckResult> {
  const entries = await deps.fetchRssFeed(channelId)

  // RSS fetch failure (empty array) → safe fallback: assume new videos exist
  if (entries.length === 0) {
    return { hasNewVideos: true, newVideoIds: [] }
  }

  const videoIds = entries.map((e) => e.videoId)
  const placeholders = videoIds.map(() => '?').join(',')
  const existingRows = deps.db
    .query(`SELECT id FROM videos WHERE id IN (${placeholders})`)
    .all(...videoIds) as { id: string }[]
  const existingIds = new Set(existingRows.map((r) => r.id))

  const newVideoIds = videoIds.filter((id) => !existingIds.has(id))

  return {
    hasNewVideos: newVideoIds.length > 0,
    newVideoIds,
  }
}

export async function checkRssForNewVideos(channelId: string): Promise<RssCheckResult> {
  return checkRssForNewVideosCore(channelId)
}
