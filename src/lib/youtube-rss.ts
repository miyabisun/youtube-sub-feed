export interface RssEntry {
  videoId: string
  title: string
  published: string
}

interface FetchRssDeps {
  fetch: typeof fetch
}

const defaultDeps: FetchRssDeps = { fetch }

export function parseAtomFeed(xml: string): RssEntry[] {
  const entries: RssEntry[] = []
  const entryRegex = /<entry>([\s\S]*?)<\/entry>/g

  let match
  while ((match = entryRegex.exec(xml)) !== null) {
    const block = match[1]
    const videoId = block.match(/<yt:videoId>([^<]+)<\/yt:videoId>/)?.[1]
    const title = block.match(/<title>([^<]+)<\/title>/)?.[1]
    const published = block.match(/<published>([^<]+)<\/published>/)?.[1]

    if (videoId) {
      entries.push({ videoId, title: title ?? '', published: published ?? '' })
    }
  }

  return entries
}

export async function fetchRssFeedCore(
  channelId: string,
  deps: FetchRssDeps = defaultDeps,
): Promise<RssEntry[]> {
  const url = `https://www.youtube.com/feeds/videos.xml?channel_id=${channelId}`
  try {
    const res = await deps.fetch(url, { signal: AbortSignal.timeout(10_000) })
    if (!res.ok) return []
    const xml = await res.text()
    return parseAtomFeed(xml)
  } catch {
    return []
  }
}

export async function fetchRssFeed(channelId: string): Promise<RssEntry[]> {
  return fetchRssFeedCore(channelId)
}
