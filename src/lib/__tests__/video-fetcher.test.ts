import { describe, test, expect } from 'bun:test'
import { Database } from 'bun:sqlite'
import { fetchChannelVideosCore } from '../video-fetcher.js'

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

function makeDeps(db: Database, overrides: Record<string, any> = {}) {
  return {
    db,
    fetchPlaylistItems: overrides.fetchPlaylistItems ?? (async () => []),
    fetchVideoDetails: overrides.fetchVideoDetails ?? (async () => []),
    fetchUUSHPlaylist: overrides.fetchUUSHPlaylist ?? (async () => []),
    notifyNewVideo: overrides.notifyNewVideo ?? (async () => {}),
    cache: overrides.cache ?? { get: () => null, set: () => {}, del: () => {}, clear: () => {}, clearPrefix: () => {} },
  }
}

describe('fetchChannelVideosCore', () => {
  test('inserts new videos and fetches their details', async () => {
    const db = setupDb()
    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => [{
        videoId: 'v1', title: 'New Video', thumbnailUrl: 'thumb', publishedAt: '2024-06-01T00:00:00Z',
      }],
      fetchVideoDetails: async () => [{
        id: 'v1', duration: 'PT5M30S', isLivestream: false, livestreamEndedAt: null,
      }],
    })

    const newIds = await fetchChannelVideosCore('UC123', 'token', {}, deps)
    expect(newIds).toEqual(['v1'])

    const video = db.query("SELECT * FROM videos WHERE id = 'v1'").get() as any
    expect(video.title).toBe('New Video')
    expect(video.duration).toBe('PT5M30S')
    expect(video.channel_id).toBe('UC123')
  })

  test('does not call Videos.list for already-known videos', async () => {
    const db = setupDb()
    db.exec("INSERT INTO videos (id, channel_id, title, duration) VALUES ('v1', 'UC123', 'Existing', 'PT3M')")

    let videoDetailsCalled = false
    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => [{
        videoId: 'v1', title: 'Existing Updated', thumbnailUrl: 'thumb', publishedAt: '2024-06-01T00:00:00Z',
      }],
      fetchVideoDetails: async () => { videoDetailsCalled = true; return [] },
    })

    const newIds = await fetchChannelVideosCore('UC123', 'token', {}, deps)
    expect(newIds).toEqual([])
    expect(videoDetailsCalled).toBe(false)
  })

  test('updates title/thumbnail for existing videos via UPSERT', async () => {
    const db = setupDb()
    db.exec("INSERT INTO videos (id, channel_id, title, thumbnail_url, duration) VALUES ('v1', 'UC123', 'Old Title', 'old_thumb', 'PT3M')")

    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => [{
        videoId: 'v1', title: 'New Title', thumbnailUrl: 'new_thumb', publishedAt: '2024-06-01T00:00:00Z',
      }],
    })

    await fetchChannelVideosCore('UC123', 'token', {}, deps)

    const video = db.query("SELECT title, thumbnail_url FROM videos WHERE id = 'v1'").get() as any
    expect(video.title).toBe('New Title')
    expect(video.thumbnail_url).toBe('new_thumb')
  })

  test('detects livestreams from video details', async () => {
    const db = setupDb()
    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => [{
        videoId: 'live1', title: 'Live Stream', thumbnailUrl: '', publishedAt: '2024-06-01T00:00:00Z',
      }],
      fetchVideoDetails: async () => [{
        id: 'live1', duration: 'PT0S', isLivestream: true, livestreamEndedAt: '2024-06-01T02:00:00Z',
      }],
    })

    await fetchChannelVideosCore('UC123', 'token', {}, deps)

    const video = db.query("SELECT is_livestream, livestream_ended_at FROM videos WHERE id = 'live1'").get() as any
    expect(video.is_livestream).toBe(1)
    expect(video.livestream_ended_at).toBe('2024-06-01T02:00:00Z')
  })

  test('returns empty for nonexistent channel', async () => {
    const db = setupDb()
    const deps = makeDeps(db)
    const newIds = await fetchChannelVideosCore('UC_NONEXISTENT', 'token', {}, deps)
    expect(newIds).toEqual([])
  })

  test('detects shorts via UUSH playlist', async () => {
    const db = setupDb()
    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => [{
        videoId: 'short1', title: 'Short Video', thumbnailUrl: '', publishedAt: '2024-06-01T00:00:00Z',
      }],
      fetchVideoDetails: async () => [{
        id: 'short1', duration: 'PT30S', isLivestream: false, livestreamEndedAt: null,
      }],
      fetchUUSHPlaylist: async () => ['short1'],
    })

    await fetchChannelVideosCore('UC123', 'token', {}, deps)

    const video = db.query("SELECT is_short FROM videos WHERE id = 'short1'").get() as any
    expect(video.is_short).toBe(1)
  })

  test('does not mark as short when not in UUSH playlist', async () => {
    const db = setupDb()
    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => [{
        videoId: 'v1', title: 'Short-ish', thumbnailUrl: '', publishedAt: '2024-06-01T00:00:00Z',
      }],
      fetchVideoDetails: async () => [{
        id: 'v1', duration: 'PT45S', isLivestream: false, livestreamEndedAt: null,
      }],
      fetchUUSHPlaylist: async () => [],
    })

    await fetchChannelVideosCore('UC123', 'token', {}, deps)

    const video = db.query("SELECT is_short FROM videos WHERE id = 'v1'").get() as any
    expect(video.is_short).toBe(0)
  })

  test('does not send Discord notifications when notify is false', async () => {
    const db = setupDb()
    let notified = false
    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => [{
        videoId: 'v1', title: 'New', thumbnailUrl: '', publishedAt: '2024-06-01T00:00:00Z',
      }],
      fetchVideoDetails: async () => [{
        id: 'v1', duration: 'PT5M', isLivestream: false, livestreamEndedAt: null,
      }],
      notifyNewVideo: async () => { notified = true },
    })

    await fetchChannelVideosCore('UC123', 'token', { notify: false }, deps)
    expect(notified).toBe(false)
  })

  test('sends Discord notifications when notify is true', async () => {
    const db = setupDb()
    let notified = false
    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => [{
        videoId: 'v1', title: 'New', thumbnailUrl: '', publishedAt: '2024-06-01T00:00:00Z',
      }],
      fetchVideoDetails: async () => [{
        id: 'v1', duration: 'PT5M', isLivestream: false, livestreamEndedAt: null,
      }],
      notifyNewVideo: async () => { notified = true },
    })

    await fetchChannelVideosCore('UC123', 'token', { notify: true }, deps)
    expect(notified).toBe(true)
  })

  test('deletes channel from DB on 404 playlistNotFound', async () => {
    const db = setupDb()
    db.exec("INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC123', 'Old Video')")

    const error = Object.assign(new Error('YouTube API error: 404'), { status: 404, reason: 'playlistNotFound' })
    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => { throw error },
    })

    const newIds = await fetchChannelVideosCore('UC123', 'token', {}, deps)
    expect(newIds).toEqual([])

    const channel = db.query("SELECT * FROM channels WHERE id = 'UC123'").get()
    expect(channel).toBeNull()

    // CASCADE should delete videos too
    const videos = db.query("SELECT * FROM videos WHERE channel_id = 'UC123'").all()
    expect(videos).toEqual([])
  })

  test('re-throws non-404 API errors', async () => {
    const db = setupDb()
    const error = Object.assign(new Error('YouTube API error: 500'), { status: 500, reason: 'backendError' })
    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => { throw error },
    })

    expect(fetchChannelVideosCore('UC123', 'token', {}, deps)).rejects.toThrow('YouTube API error: 500')
  })

  test('updates last_fetched_at even when no new videos', async () => {
    const db = setupDb()
    db.exec("INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC123', 'Existing')")

    const deps = makeDeps(db, {
      fetchPlaylistItems: async () => [{
        videoId: 'v1', title: 'Existing', thumbnailUrl: '', publishedAt: '2024-06-01T00:00:00Z',
      }],
    })

    await fetchChannelVideosCore('UC123', 'token', {}, deps)

    const channel = db.query("SELECT last_fetched_at FROM channels WHERE id = 'UC123'").get() as any
    expect(channel.last_fetched_at).not.toBeNull()
  })
})
