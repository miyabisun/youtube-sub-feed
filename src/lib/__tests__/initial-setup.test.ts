import { describe, test, expect } from 'bun:test'
import { Database } from 'bun:sqlite'
import { runInitialSetupCore } from '../initial-setup.js'

function setupDb() {
  const sqlite = new Database(':memory:')
  sqlite.exec('PRAGMA foreign_keys = ON')
  sqlite.exec(`CREATE TABLE channels (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, thumbnail_url TEXT,
    upload_playlist_id TEXT, show_livestreams INTEGER NOT NULL DEFAULT 0,
    fast_polling INTEGER NOT NULL DEFAULT 0, last_fetched_at TEXT, created_at TEXT NOT NULL
  )`)
  sqlite.exec(`CREATE TABLE videos (
    id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, title TEXT NOT NULL,
    thumbnail_url TEXT, published_at TEXT, duration TEXT, is_short INTEGER NOT NULL DEFAULT 0,
    is_livestream INTEGER NOT NULL DEFAULT 0, livestream_ended_at TEXT,
    is_hidden INTEGER NOT NULL DEFAULT 0, fetched_at TEXT,
    FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
  )`)
  return sqlite
}

function makeDeps(db: Database, overrides: Record<string, any> = {}) {
  return {
    db,
    getValidAccessToken: overrides.getValidAccessToken ?? (async () => 'token'),
    syncSubscriptions: overrides.syncSubscriptions ?? (async () => ({ added: 0, removed: 0 })),
    fetchChannelVideos: overrides.fetchChannelVideos ?? (async () => []),
    notifySetupComplete: overrides.notifySetupComplete ?? (async () => {}),
  }
}

describe('runInitialSetupCore', () => {
  test('skips when channels already exist', async () => {
    const db = setupDb()
    db.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Existing', '2024-01-01')")

    let syncCalled = false
    const deps = makeDeps(db, {
      syncSubscriptions: async () => { syncCalled = true },
    })

    await runInitialSetupCore(deps)
    expect(syncCalled).toBe(false)
  })

  test('skips when no valid token', async () => {
    const db = setupDb()
    let syncCalled = false
    const deps = makeDeps(db, {
      getValidAccessToken: async () => null,
      syncSubscriptions: async () => { syncCalled = true },
    })

    await runInitialSetupCore(deps)
    expect(syncCalled).toBe(false)
  })

  test('syncs subscriptions and fetches videos for all channels', async () => {
    const db = setupDb()
    let syncCalled = false
    let fetchedChannels: string[] = []
    let notifyArgs: { channels: number; videos: number } | null = null

    const deps = makeDeps(db, {
      syncSubscriptions: async () => {
        syncCalled = true
        db.exec("INSERT INTO channels (id, title, upload_playlist_id, created_at) VALUES ('UC1', 'Ch1', 'UU1', '2024-01-01')")
        db.exec("INSERT INTO channels (id, title, upload_playlist_id, created_at) VALUES ('UC2', 'Ch2', 'UU2', '2024-01-01')")
      },
      fetchChannelVideos: async (channelId: string) => {
        fetchedChannels.push(channelId)
        db.exec(`INSERT INTO videos (id, channel_id, title) VALUES ('v_${channelId}', '${channelId}', 'Video')`)
        return [`v_${channelId}`]
      },
      notifySetupComplete: async (channels: number, videos: number) => {
        notifyArgs = { channels, videos }
      },
    })

    await runInitialSetupCore(deps)

    expect(syncCalled).toBe(true)
    expect(fetchedChannels).toEqual(['UC1', 'UC2'])
    expect(notifyArgs).toEqual({ channels: 2, videos: 2 })
  })

  test('continues fetching when one channel fails', async () => {
    const db = setupDb()
    let fetchedChannels: string[] = []

    const deps = makeDeps(db, {
      syncSubscriptions: async () => {
        db.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Ch1', '2024-01-01')")
        db.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC2', 'Ch2', '2024-01-01')")
      },
      fetchChannelVideos: async (channelId: string) => {
        if (channelId === 'UC1') throw new Error('API error')
        fetchedChannels.push(channelId)
        return []
      },
    })

    await runInitialSetupCore(deps)

    expect(fetchedChannels).toEqual(['UC2'])
  })

  test('passes notify: false to fetchChannelVideos', async () => {
    const db = setupDb()
    let receivedOpts: any = null

    const deps = makeDeps(db, {
      syncSubscriptions: async () => {
        db.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Ch1', '2024-01-01')")
      },
      fetchChannelVideos: async (_id: string, _token: string, opts: any) => {
        receivedOpts = opts
        return []
      },
    })

    await runInitialSetupCore(deps)
    expect(receivedOpts).toEqual({ notify: false })
  })
})
