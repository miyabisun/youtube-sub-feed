import { sqlite } from '../db/index.js'
import { getValidAccessToken } from './token-manager.js'
import { syncSubscriptions } from './channel-sync.js'
import { fetchChannelVideos } from './video-fetcher.js'
import { notifySetupComplete } from './discord.js'
import type { Database } from 'bun:sqlite'

interface SetupDeps {
  db: Database
  getValidAccessToken: () => Promise<string | null>
  syncSubscriptions: (token: string) => Promise<any>
  fetchChannelVideos: (channelId: string, token: string, opts?: { notify?: boolean }) => Promise<string[]>
  notifySetupComplete: (channelCount: number, videoCount: number) => Promise<void>
}

const defaultDeps: SetupDeps = {
  db: sqlite,
  getValidAccessToken,
  syncSubscriptions,
  fetchChannelVideos,
  notifySetupComplete,
}

export async function runInitialSetupCore(deps: SetupDeps = defaultDeps): Promise<void> {
  const { db } = deps

  const channelCount = db.query('SELECT COUNT(*) as count FROM channels').get() as { count: number }
  if (channelCount.count > 0) {
    console.log('[setup] Channels already exist, skipping initial setup')
    return
  }

  console.log('[setup] Starting initial setup...')

  const token = await deps.getValidAccessToken()
  if (!token) {
    console.log('[setup] No valid token, cannot run initial setup')
    return
  }

  // Sync subscriptions first
  await deps.syncSubscriptions(token)

  // Fetch videos for all channels
  const channels = db.query('SELECT id FROM channels').all() as { id: string }[]
  console.log(`[setup] Fetching videos for ${channels.length} channels...`)

  for (const channel of channels) {
    try {
      await deps.fetchChannelVideos(channel.id, token, { notify: false })
    } catch (e) {
      console.error(`[setup] Error fetching ${channel.id}:`, e)
    }
  }

  const videoCount = db.query('SELECT COUNT(*) as count FROM videos').get() as { count: number }
  console.log('[setup] Initial setup complete')

  deps.notifySetupComplete(channels.length, videoCount.count).catch(() => {})
}

export async function runInitialSetup(): Promise<void> {
  return runInitialSetupCore()
}
