import { describe, test, expect } from 'bun:test'
import { parseAtomFeed, fetchRssFeedCore } from '../youtube-rss.js'

describe('parseAtomFeed', () => {
  test('extracts video entries from valid Atom XML', () => {
    const xml = `<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns="http://www.w3.org/2005/Atom">
  <title>Channel Title</title>
  <entry>
    <yt:videoId>abc123</yt:videoId>
    <title>First Video</title>
    <published>2024-06-01T12:00:00+00:00</published>
  </entry>
  <entry>
    <yt:videoId>def456</yt:videoId>
    <title>Second Video</title>
    <published>2024-06-02T08:30:00+00:00</published>
  </entry>
</feed>`

    const entries = parseAtomFeed(xml)
    expect(entries).toEqual([
      { videoId: 'abc123', title: 'First Video', published: '2024-06-01T12:00:00+00:00' },
      { videoId: 'def456', title: 'Second Video', published: '2024-06-02T08:30:00+00:00' },
    ])
  })

  test('returns empty array for feed with no entries', () => {
    const xml = `<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns="http://www.w3.org/2005/Atom">
  <title>Empty Channel</title>
</feed>`

    expect(parseAtomFeed(xml)).toEqual([])
  })

  test('returns empty array for invalid XML', () => {
    expect(parseAtomFeed('not xml at all')).toEqual([])
    expect(parseAtomFeed('')).toEqual([])
  })

  test('skips entries missing videoId', () => {
    const xml = `<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <title>No Video ID</title>
    <published>2024-06-01T12:00:00+00:00</published>
  </entry>
  <entry>
    <yt:videoId>valid1</yt:videoId>
    <title>Has Video ID</title>
    <published>2024-06-02T00:00:00+00:00</published>
  </entry>
</feed>`

    const entries = parseAtomFeed(xml)
    expect(entries).toEqual([
      { videoId: 'valid1', title: 'Has Video ID', published: '2024-06-02T00:00:00+00:00' },
    ])
  })
})

describe('fetchRssFeedCore', () => {
  test('fetches and parses RSS for a channel', async () => {
    const xml = `<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <yt:videoId>vid1</yt:videoId>
    <title>Test</title>
    <published>2024-06-01T00:00:00+00:00</published>
  </entry>
</feed>`

    const deps = {
      fetch: async () => new Response(xml, { status: 200 }),
    }

    const entries = await fetchRssFeedCore('UC123', deps)
    expect(entries).toEqual([
      { videoId: 'vid1', title: 'Test', published: '2024-06-01T00:00:00+00:00' },
    ])
  })

  test('constructs correct RSS URL', async () => {
    let requestedUrl = ''
    const deps = {
      fetch: async (url: string | URL | Request) => {
        requestedUrl = typeof url === 'string' ? url : url.toString()
        return new Response('<feed></feed>', { status: 200 })
      },
    }

    await fetchRssFeedCore('UCabc123', deps)
    expect(requestedUrl).toBe('https://www.youtube.com/feeds/videos.xml?channel_id=UCabc123')
  })

  test('returns empty array on HTTP error', async () => {
    const deps = {
      fetch: async () => new Response('Not Found', { status: 404 }),
    }

    const entries = await fetchRssFeedCore('UC123', deps)
    expect(entries).toEqual([])
  })

  test('returns empty array on network error', async () => {
    const deps = {
      fetch: async () => { throw new Error('Network failure') },
    }

    const entries = await fetchRssFeedCore('UC123', deps)
    expect(entries).toEqual([])
  })

  test('passes abort signal with 10s timeout', async () => {
    let receivedSignal: AbortSignal | undefined
    const deps = {
      fetch: async (_url: string | URL | Request, init?: RequestInit) => {
        receivedSignal = init?.signal ?? undefined
        return new Response('<feed></feed>', { status: 200 })
      },
    }

    await fetchRssFeedCore('UC123', deps)
    expect(receivedSignal).toBeInstanceOf(AbortSignal)
  })
})
