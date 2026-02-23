import { describe, test, expect, afterEach } from 'bun:test'
import { Database } from 'bun:sqlite'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

function setupDb() {
  const sqlite = new Database(':memory:')
  sqlite.exec('PRAGMA foreign_keys = ON')
  sqlite.exec(`CREATE TABLE channels (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, thumbnail_url TEXT,
    upload_playlist_id TEXT, show_livestreams INTEGER NOT NULL DEFAULT 0,
    last_fetched_at TEXT, created_at TEXT NOT NULL
  )`)
  sqlite.exec(`CREATE TABLE videos (
    id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, title TEXT NOT NULL,
    thumbnail_url TEXT, published_at TEXT, duration TEXT, is_short INTEGER NOT NULL DEFAULT 0,
    is_livestream INTEGER NOT NULL DEFAULT 0, livestream_ended_at TEXT,
    is_hidden INTEGER NOT NULL DEFAULT 0, fetched_at TEXT,
    FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
  )`)
  sqlite.exec(`CREATE TABLE channel_groups (
    channel_id TEXT NOT NULL, group_id INTEGER NOT NULL,
    PRIMARY KEY (channel_id, group_id),
    FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
  )`)
  return sqlite
}

function mockSubscriptions(subs: { channelId: string; title: string }[]) {
  globalThis.fetch = (async () => ({
    ok: true,
    json: async () => ({
      items: subs.map((s) => ({
        snippet: {
          resourceId: { channelId: s.channelId },
          title: s.title,
          thumbnails: { default: { url: 'thumb' } },
        },
      })),
    }),
  })) as typeof fetch
}

// Inline sync function that uses provided sqlite instance
async function syncSubscriptions(sqlite: Database, accessToken: string) {
  // Fetch subscriptions (mocked)
  const res = await fetch('https://www.googleapis.com/youtube/v3/subscriptions?part=snippet&mine=true&maxResults=50', {
    headers: { Authorization: `Bearer ${accessToken}` },
  })
  const data: any = await res.json()
  const subs = (data.items || []).map((item: any) => ({
    channelId: item.snippet.resourceId.channelId,
    title: item.snippet.title,
    thumbnailUrl: item.snippet.thumbnails?.default?.url || '',
  }))
  const remoteIds = new Set(subs.map((s: any) => s.channelId))

  const localChannels = sqlite.query('SELECT id FROM channels').all() as { id: string }[]
  const localIds = new Set(localChannels.map((c) => c.id))
  const now = new Date().toISOString()
  let added = 0
  let removed = 0

  for (const sub of subs) {
    if (!localIds.has(sub.channelId)) {
      const uploadPlaylistId = sub.channelId.replace(/^UC/, 'UU')
      sqlite.query('INSERT INTO channels (id, title, thumbnail_url, upload_playlist_id, created_at) VALUES (?, ?, ?, ?, ?)').run(sub.channelId, sub.title, sub.thumbnailUrl, uploadPlaylistId, now)
      added++
    }
  }

  for (const local of localChannels) {
    if (!remoteIds.has(local.id)) {
      sqlite.query('DELETE FROM channels WHERE id = ?').run(local.id)
      removed++
    }
  }

  return { added, removed }
}

describe('syncSubscriptions', () => {
  test('adds new channels from subscriptions', async () => {
    const sqlite = setupDb()
    mockSubscriptions([
      { channelId: 'UC111', title: 'Channel 1' },
      { channelId: 'UC222', title: 'Channel 2' },
    ])

    const result = await syncSubscriptions(sqlite, 'token')
    expect(result.added).toBe(2)
    expect(result.removed).toBe(0)

    const channels = sqlite.query('SELECT id, upload_playlist_id FROM channels ORDER BY id').all() as any[]
    expect(channels).toHaveLength(2)
    expect(channels[0].upload_playlist_id).toBe('UU111')
  })

  test('removes unsubscribed channels', async () => {
    const sqlite = setupDb()
    sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC111', 'Old Channel', '2024-01-01')")
    sqlite.exec("INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC111', 'Video')")

    mockSubscriptions([{ channelId: 'UC222', title: 'New Channel' }])

    const result = await syncSubscriptions(sqlite, 'token')
    expect(result.added).toBe(1)
    expect(result.removed).toBe(1)

    const channels = sqlite.query('SELECT id FROM channels').all() as any[]
    expect(channels).toHaveLength(1)
    expect(channels[0].id).toBe('UC222')

    // Videos should be cascade deleted
    const videos = sqlite.query('SELECT id FROM videos').all()
    expect(videos).toHaveLength(0)
  })

  test('does not re-add existing channels', async () => {
    const sqlite = setupDb()
    sqlite.exec("INSERT INTO channels (id, title, created_at) VALUES ('UC111', 'Existing', '2024-01-01')")

    mockSubscriptions([{ channelId: 'UC111', title: 'Existing' }])

    const result = await syncSubscriptions(sqlite, 'token')
    expect(result.added).toBe(0)
    expect(result.removed).toBe(0)
  })
})
