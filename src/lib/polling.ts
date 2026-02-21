import { sqlite } from '../db/index.js'
import { getValidAccessToken } from './token-manager.js'
import { isQuotaExceeded, getQuotaResetTime } from './quota-manager.js'
import { fetchChannelVideos } from './video-fetcher.js'
import { checkLivestreams } from './livestream-checker.js'
import { syncSubscriptions } from './channel-sync.js'
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
      const token = await waitForToken()
      await waitForQuota()

      const channels = sqlite.query(
        "SELECT id FROM channels WHERE fast_polling = 0 ORDER BY last_fetched_at ASC NULLS FIRST"
      ).all() as { id: string }[]

      const count = channels.length
      if (count === 0) {
        setTimeout(tick, 60_000)
        return
      }

      index = index % count
      const { id } = channels[index]

      const newVideoIds = await fetchChannelVideos(id, token)
      if (newVideoIds.length > 0) {
        console.log(`[polling] Normal: ${id} - ${newVideoIds.length} new videos`)
      }

      index++
      if (index >= count) {
        // One cycle complete, clear UUSH cache only
        cache.clearPrefix('uush:')
        index = 0
      }

      const interval = Math.floor(NORMAL_INTERVAL_MS / count)
      setTimeout(tick, interval)
    } catch (e) {
      console.error('[polling] Normal error:', e)
      setTimeout(tick, 60_000)
    }
  }

  console.log('[polling] Starting normal polling (30min/cycle)')
  tick()
}

export function startFastPolling(): void {
  let index = 0

  async function tick() {
    try {
      const token = await waitForToken()
      await waitForQuota()

      const channels = sqlite.query(
        "SELECT id FROM channels WHERE fast_polling = 1 ORDER BY last_fetched_at ASC NULLS FIRST"
      ).all() as { id: string }[]

      const count = channels.length
      if (count === 0) {
        setTimeout(tick, 60_000)
        return
      }

      index = index % count
      const { id } = channels[index]

      const newVideoIds = await fetchChannelVideos(id, token)
      if (newVideoIds.length > 0) {
        console.log(`[polling] Fast: ${id} - ${newVideoIds.length} new videos`)
      }

      // Check livestreams during fast polling
      await checkLivestreams(token)

      index++
      if (index >= count) index = 0

      const interval = Math.floor(FAST_INTERVAL_MS / count)
      setTimeout(tick, interval)
    } catch (e) {
      console.error('[polling] Fast error:', e)
      setTimeout(tick, 60_000)
    }
  }

  console.log('[polling] Starting fast polling (10min/cycle)')
  tick()
}

export function startDailySync(): void {
  async function sync() {
    try {
      const token = await waitForToken()
      await syncSubscriptions(token)
    } catch (e) {
      console.error('[polling] Daily sync error:', e)
    }
    // Schedule next sync in 24 hours
    setTimeout(sync, 24 * 60 * 60 * 1000)
  }

  console.log('[polling] Starting daily subscription sync')
  sync()
}
