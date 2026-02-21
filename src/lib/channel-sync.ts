import { sqlite } from '../db/index.js'
import { fetchSubscriptions } from './youtube-api.js'

export async function syncSubscriptions(accessToken: string): Promise<{ added: number; removed: number }> {
  const subs = await fetchSubscriptions(accessToken)
  const remoteIds = new Set(subs.map((s) => s.channelId))

  const localChannels = sqlite.query('SELECT id FROM channels').all() as { id: string }[]
  const localIds = new Set(localChannels.map((c) => c.id))

  const now = new Date().toISOString()
  let added = 0
  let removed = 0

  // Add new channels
  const insertStmt = sqlite.query('INSERT INTO channels (id, title, thumbnail_url, upload_playlist_id, created_at) VALUES (?, ?, ?, ?, ?)')
  for (const sub of subs) {
    if (!localIds.has(sub.channelId)) {
      const uploadPlaylistId = sub.channelId.replace(/^UC/, 'UU')
      insertStmt.run(sub.channelId, sub.title, sub.thumbnailUrl, uploadPlaylistId, now)
      added++
    }
  }

  // Remove unsubscribed channels (CASCADE deletes videos, channel_groups)
  const deleteStmt = sqlite.query('DELETE FROM channels WHERE id = ?')
  for (const local of localChannels) {
    if (!remoteIds.has(local.id)) {
      deleteStmt.run(local.id)
      removed++
    }
  }

  console.log(`[sync] Subscriptions synced: +${added} -${removed} (total: ${remoteIds.size})`)
  return { added, removed }
}
