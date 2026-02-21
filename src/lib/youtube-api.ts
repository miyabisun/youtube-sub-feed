import { withRetry } from './youtube-retry.js'
import { QuotaExceededError } from './quota-manager.js'

const YOUTUBE_API_BASE = 'https://www.googleapis.com/youtube/v3'

class YouTubeApiError extends Error {
  status: number
  reason?: string
  constructor(status: number, message: string, reason?: string) {
    super(message)
    this.name = 'YouTubeApiError'
    this.status = status
    this.reason = reason
  }
}

async function youtubeGet(path: string, params: Record<string, string>, accessToken: string): Promise<any> {
  const url = new URL(`${YOUTUBE_API_BASE}/${path}`)
  for (const [k, v] of Object.entries(params)) url.searchParams.set(k, v)

  const res = await fetch(url.toString(), {
    headers: { Authorization: `Bearer ${accessToken}` },
  })

  if (!res.ok) {
    const body = await res.json().catch(() => ({}))
    const reason = body?.error?.errors?.[0]?.reason
    if (res.status === 403 && reason === 'quotaExceeded') {
      throw new YouTubeApiError(403, 'Quota exceeded', 'quotaExceeded')
    }
    throw new YouTubeApiError(res.status, `YouTube API error: ${res.status}`, reason)
  }

  return res.json()
}

export interface Subscription {
  channelId: string
  title: string
  thumbnailUrl: string
}

export async function fetchSubscriptions(accessToken: string): Promise<Subscription[]> {
  const results: Subscription[] = []
  let pageToken: string | undefined

  do {
    const params: Record<string, string> = {
      part: 'snippet',
      mine: 'true',
      maxResults: '50',
    }
    if (pageToken) params.pageToken = pageToken

    const data = await withRetry(() => youtubeGet('subscriptions', params, accessToken))

    for (const item of data.items || []) {
      results.push({
        channelId: item.snippet.resourceId.channelId,
        title: item.snippet.title,
        thumbnailUrl: item.snippet.thumbnails?.default?.url || '',
      })
    }

    pageToken = data.nextPageToken
  } while (pageToken)

  return results
}

export interface PlaylistItem {
  videoId: string
  title: string
  thumbnailUrl: string
  publishedAt: string
}

export async function fetchPlaylistItems(playlistId: string, accessToken: string, maxResults = 10): Promise<PlaylistItem[]> {
  const data = await withRetry(() =>
    youtubeGet('playlistItems', {
      part: 'snippet',
      playlistId,
      maxResults: String(maxResults),
    }, accessToken)
  )

  return (data.items || []).map((item: any) => ({
    videoId: item.snippet.resourceId.videoId,
    title: item.snippet.title,
    thumbnailUrl: item.snippet.thumbnails?.medium?.url || item.snippet.thumbnails?.default?.url || '',
    publishedAt: item.snippet.publishedAt,
  }))
}

export interface VideoDetails {
  id: string
  duration: string
  isLivestream: boolean
  livestreamEndedAt: string | null
}

export async function fetchVideoDetails(videoIds: string[], accessToken: string): Promise<VideoDetails[]> {
  if (videoIds.length === 0) return []

  // Batch in groups of 50
  const results: VideoDetails[] = []
  for (let i = 0; i < videoIds.length; i += 50) {
    const batch = videoIds.slice(i, i + 50)
    const data = await withRetry(() =>
      youtubeGet('videos', {
        part: 'contentDetails,liveStreamingDetails',
        id: batch.join(','),
      }, accessToken)
    )

    for (const item of data.items || []) {
      results.push({
        id: item.id,
        duration: item.contentDetails?.duration || 'PT0S',
        isLivestream: !!item.liveStreamingDetails,
        livestreamEndedAt: item.liveStreamingDetails?.actualEndTime || null,
      })
    }
  }

  return results
}

export async function fetchUUSHPlaylist(channelId: string, accessToken: string): Promise<string[]> {
  const uushId = channelId.replace(/^UC/, 'UUSH')

  try {
    const items = await fetchPlaylistItems(uushId, accessToken, 50)
    return items.map((item) => item.videoId)
  } catch {
    // UUSH is unofficial, failure is expected
    return []
  }
}
