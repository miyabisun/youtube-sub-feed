import { describe, test, expect, afterEach, mock } from 'bun:test'
import { fetchSubscriptions, fetchPlaylistItems, fetchVideoDetails, fetchUUSHPlaylist } from '../youtube-api.js'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

function mockFetch(handler: (url: string) => any) {
  globalThis.fetch = (async (input: any) => {
    const url = typeof input === 'string' ? input : input.url
    const body = handler(url)
    return { ok: true, json: async () => body } as Response
  }) as typeof fetch
}

describe('fetchSubscriptions', () => {
  test('parses subscription list response', async () => {
    mockFetch(() => ({
      items: [
        { snippet: { resourceId: { channelId: 'UC1' }, title: 'Ch1', thumbnails: { default: { url: 'thumb1' } } } },
        { snippet: { resourceId: { channelId: 'UC2' }, title: 'Ch2', thumbnails: { default: { url: 'thumb2' } } } },
      ],
    }))

    const subs = await fetchSubscriptions('token')
    expect(subs).toHaveLength(2)
    expect(subs[0].channelId).toBe('UC1')
    expect(subs[0].title).toBe('Ch1')
    expect(subs[1].channelId).toBe('UC2')
  })

  test('handles pagination', async () => {
    let calls = 0
    globalThis.fetch = (async (input: any) => {
      calls++
      const url = typeof input === 'string' ? input : input.url
      if (!url.includes('pageToken')) {
        return { ok: true, json: async () => ({ items: [{ snippet: { resourceId: { channelId: 'UC1' }, title: 'Ch1', thumbnails: {} } }], nextPageToken: 'page2' }) } as Response
      }
      return { ok: true, json: async () => ({ items: [{ snippet: { resourceId: { channelId: 'UC2' }, title: 'Ch2', thumbnails: {} } }] }) } as Response
    }) as typeof fetch

    const subs = await fetchSubscriptions('token')
    expect(subs).toHaveLength(2)
    expect(calls).toBe(2)
  })
})

describe('fetchPlaylistItems', () => {
  test('parses playlist items response', async () => {
    mockFetch(() => ({
      items: [
        { snippet: { resourceId: { videoId: 'v1' }, title: 'Video 1', thumbnails: { medium: { url: 'thumb1' } }, publishedAt: '2024-01-01T00:00:00Z' } },
      ],
    }))

    const items = await fetchPlaylistItems('UU123', 'token')
    expect(items).toHaveLength(1)
    expect(items[0].videoId).toBe('v1')
    expect(items[0].title).toBe('Video 1')
    expect(items[0].publishedAt).toBe('2024-01-01T00:00:00Z')
  })
})

describe('fetchVideoDetails', () => {
  test('returns empty array for empty input', async () => {
    const result = await fetchVideoDetails([], 'token')
    expect(result).toHaveLength(0)
  })

  test('parses video details with duration and livestream info', async () => {
    mockFetch(() => ({
      items: [
        { id: 'v1', contentDetails: { duration: 'PT5M30S' }, liveStreamingDetails: undefined },
        { id: 'v2', contentDetails: { duration: 'PT1H' }, liveStreamingDetails: { actualEndTime: '2024-01-01T02:00:00Z' } },
      ],
    }))

    const details = await fetchVideoDetails(['v1', 'v2'], 'token')
    expect(details).toHaveLength(2)
    expect(details[0].duration).toBe('PT5M30S')
    expect(details[0].isLivestream).toBe(false)
    expect(details[1].isLivestream).toBe(true)
    expect(details[1].livestreamEndedAt).toBe('2024-01-01T02:00:00Z')
  })

  test('detects live stream without end time', async () => {
    mockFetch(() => ({
      items: [
        { id: 'v1', contentDetails: { duration: 'PT0S' }, liveStreamingDetails: {} },
      ],
    }))

    const details = await fetchVideoDetails(['v1'], 'token')
    expect(details[0].isLivestream).toBe(true)
    expect(details[0].livestreamEndedAt).toBeNull()
  })
})

describe('fetchUUSHPlaylist', () => {
  test('converts UC to UUSH and returns video IDs', async () => {
    let requestedUrl = ''
    globalThis.fetch = (async (input: any) => {
      requestedUrl = typeof input === 'string' ? input : input.url
      return { ok: true, json: async () => ({ items: [{ snippet: { resourceId: { videoId: 'sv1' }, title: 'Short 1', thumbnails: {}, publishedAt: '2024-01-01T00:00:00Z' } }] }) } as Response
    }) as typeof fetch

    const ids = await fetchUUSHPlaylist('UC123abc', 'token')
    expect(requestedUrl).toContain('UUSH123abc')
    expect(ids).toContain('sv1')
  })

  test('returns empty array on failure', async () => {
    globalThis.fetch = (async () => {
      return { ok: false, json: async () => ({ error: { errors: [{ reason: 'notFound' }] } }) } as Response
    }) as typeof fetch

    const ids = await fetchUUSHPlaylist('UC123', 'token')
    expect(ids).toHaveLength(0)
  })
})
