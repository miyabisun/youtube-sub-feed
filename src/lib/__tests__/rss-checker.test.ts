import { describe, test, expect } from 'bun:test'
import { Database } from 'bun:sqlite'
import { checkRssForNewVideosCore } from '../rss-checker.js'
import type { RssEntry } from '../youtube-rss.js'

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
  sqlite.exec("INSERT INTO channels (id, title, upload_playlist_id, created_at) VALUES ('UC123', 'Test Ch', 'UU123', '2024-01-01')")
  return sqlite
}

function makeDeps(db: Database, rssEntries: RssEntry[]) {
  return {
    db,
    fetchRssFeed: async (_channelId: string) => rssEntries,
  }
}

describe('checkRssForNewVideosCore', () => {
  test('returns hasNewVideos=false when all RSS videos are in DB', async () => {
    const db = setupDb()
    db.exec("INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC123', 'Video 1')")
    db.exec("INSERT INTO videos (id, channel_id, title) VALUES ('v2', 'UC123', 'Video 2')")

    const deps = makeDeps(db, [
      { videoId: 'v1', title: 'Video 1', published: '2024-06-01T00:00:00Z' },
      { videoId: 'v2', title: 'Video 2', published: '2024-06-02T00:00:00Z' },
    ])

    const result = await checkRssForNewVideosCore('UC123', deps)
    expect(result.hasNewVideos).toBe(false)
    expect(result.newVideoIds).toEqual([])
  })

  test('returns hasNewVideos=true with new video IDs when unknown videos exist', async () => {
    const db = setupDb()
    db.exec("INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC123', 'Video 1')")

    const deps = makeDeps(db, [
      { videoId: 'v1', title: 'Video 1', published: '2024-06-01T00:00:00Z' },
      { videoId: 'v2', title: 'New Video', published: '2024-06-02T00:00:00Z' },
      { videoId: 'v3', title: 'Another New', published: '2024-06-03T00:00:00Z' },
    ])

    const result = await checkRssForNewVideosCore('UC123', deps)
    expect(result.hasNewVideos).toBe(true)
    expect(result.newVideoIds).toEqual(['v2', 'v3'])
  })

  test('returns hasNewVideos=true when RSS fetch fails (empty array = safe fallback)', async () => {
    const db = setupDb()
    db.exec("INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC123', 'Video 1')")

    const deps = makeDeps(db, [])

    const result = await checkRssForNewVideosCore('UC123', deps)
    expect(result.hasNewVideos).toBe(true)
    expect(result.newVideoIds).toEqual([])
  })

  test('returns all videos as new when DB has no videos', async () => {
    const db = setupDb()

    const deps = makeDeps(db, [
      { videoId: 'v1', title: 'Video 1', published: '2024-06-01T00:00:00Z' },
      { videoId: 'v2', title: 'Video 2', published: '2024-06-02T00:00:00Z' },
    ])

    const result = await checkRssForNewVideosCore('UC123', deps)
    expect(result.hasNewVideos).toBe(true)
    expect(result.newVideoIds).toEqual(['v1', 'v2'])
  })
})
