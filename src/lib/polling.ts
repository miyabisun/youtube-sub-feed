import { sqlite } from '../db/index.js'
import { getValidAccessToken } from './token-manager.js'
import { isQuotaExceeded, getQuotaResetTime } from './quota-manager.js'
import { fetchChannelVideos } from './video-fetcher.js'
import { checkLivestreams } from './livestream-checker.js'
import { syncSubscriptions } from './channel-sync.js'
import { checkRssForNewVideos } from './rss-checker.js'
import cache from './cache.js'

const NORMAL_INTERVAL_MS = 30 * 60 * 1000 // 30 minutes per cycle
const FAST_INTERVAL_MS = 10 * 60 * 1000   // 10 minutes per cycle

async function waitForToken(): Promise<string> {
  while (true) {
    const token = await getValidAccessToken()
    if (token) return token
    console.log('[polling] No valid token, waiting 60s...')
    await new Promise((r) => setTimeout(r, 60_000))
  }
}

async function waitForQuota(): Promise<void> {
  if (!isQuotaExceeded()) return
  const resetTime = getQuotaResetTime()
  const waitMs = resetTime ? resetTime - Date.now() : 60_000
  if (waitMs > 0) {
    console.log(`[polling] Quota exceeded, waiting ${Math.ceil(waitMs / 60000)}min...`)
    await new Promise((r) => setTimeout(r, waitMs))
  }
}

export function startNormalPolling(): void {
  let index = 0

  async function tick() {
    try {
      const channels = sqlite.query(
        "SELECT id, last_fetched_at FROM channels WHERE fast_polling = 0 ORDER BY last_fetched_at ASC NULLS FIRST"
      ).all() as { id: string; last_fetched_at: string | null }[]

      const count = channels.length
      if (count === 0) {
        setTimeout(tick, 60_000)
        return
      }

      index = index % count
      const channel = channels[index]

      const advance = () => {
        index++
        if (index >= count) {
          cache.clearPrefix('uush:')
          index = 0
        }
        setTimeout(tick, Math.floor(NORMAL_INTERVAL_MS / count))
      }

      // RSS-first: skip API if no new videos detected (only for previously fetched channels)
      if (channel.last_fetched_at !== null) {
        const rss = await checkRssForNewVideos(channel.id)
        if (!rss.hasNewVideos) {
          const now = new Date().toISOString()
          sqlite.query('UPDATE channels SET last_fetched_at = ? WHERE id = ?').run(now, channel.id)
          advance()
          return
        }
      }

      const token = await waitForToken()
      await waitForQuota()

      // Skip notifications for first-time fetches
      const notify = channel.last_fetched_at !== null
      const newVideoIds = await fetchChannelVideos(channel.id, token, { notify })
      if (newVideoIds.length > 0) {
        console.log(`[polling] Normal: ${channel.id} - ${newVideoIds.length} new videos`)
      }

      advance()
    } catch (e) {
      console.error('[polling] Normal error:', e)
      index++
      setTimeout(tick, 60_000)
    }
  }

  console.log('[polling] Starting normal polling (30min/cycle, RSS-first)')
  tick()
}

export function startFastPolling(): void {
  let index = 0

  async function tick() {
    try {
      const channels = sqlite.query(
        "SELECT id, last_fetched_at FROM channels WHERE fast_polling = 1 ORDER BY last_fetched_at ASC NULLS FIRST"
      ).all() as { id: string; last_fetched_at: string | null }[]

      const count = channels.length
      if (count === 0) {
        setTimeout(tick, 60_000)
        return
      }

      index = index % count
      const channel = channels[index]

      const advance = () => {
        index++
        if (index >= count) index = 0
        setTimeout(tick, Math.floor(FAST_INTERVAL_MS / count))
      }

      // RSS-first: skip API if no new videos detected (only for previously fetched channels)
      if (channel.last_fetched_at !== null) {
        const rss = await checkRssForNewVideos(channel.id)
        if (!rss.hasNewVideos) {
          const now = new Date().toISOString()
          sqlite.query('UPDATE channels SET last_fetched_at = ? WHERE id = ?').run(now, channel.id)

          // Check livestreams even when RSS skips API (non-blocking: skip if no token)
          const token = await getValidAccessToken()
          if (token) await checkLivestreams(token)

          advance()
          return
        }
      }

      const token = await waitForToken()
      await waitForQuota()

      const notify = channel.last_fetched_at !== null
      const newVideoIds = await fetchChannelVideos(channel.id, token, { notify })
      if (newVideoIds.length > 0) {
        console.log(`[polling] Fast: ${channel.id} - ${newVideoIds.length} new videos`)
      }

      // Check livestreams during fast polling
      await checkLivestreams(token)

      advance()
    } catch (e) {
      console.error('[polling] Fast error:', e)
      index++
      setTimeout(tick, 60_000)
    }
  }

  console.log('[polling] Starting fast polling (10min/cycle, RSS-first)')
  tick()
}

const SYNC_INTERVAL_MS = 10 * 60 * 1000 // 10 minutes

export function startPeriodicSync(): void {
  async function sync() {
    try {
      const token = await waitForToken()
      await syncSubscriptions(token)
    } catch (e) {
      console.error('[polling] Sync error:', e)
    }
    setTimeout(sync, SYNC_INTERVAL_MS)
  }

  console.log('[polling] Starting periodic subscription sync (10min)')
  sync()
}
