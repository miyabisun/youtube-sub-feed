import { sqlite } from '../db/index.js'
import { fetchVideoDetails } from './youtube-api.js'

export async function checkLivestreams(accessToken: string): Promise<void> {
  const liveVideos = sqlite.query(
    "SELECT id FROM videos WHERE is_livestream = 1 AND livestream_ended_at IS NULL"
  ).all() as { id: string }[]

  if (liveVideos.length === 0) return

  const videoIds = liveVideos.map((v) => v.id)
  const details = await fetchVideoDetails(videoIds, accessToken)

  const updateStmt = sqlite.query('UPDATE videos SET livestream_ended_at = ? WHERE id = ?')
  for (const detail of details) {
    if (detail.livestreamEndedAt) {
      updateStmt.run(detail.livestreamEndedAt, detail.id)
      console.log(`[livestream] ${detail.id} ended at ${detail.livestreamEndedAt}`)
    }
  }
}
