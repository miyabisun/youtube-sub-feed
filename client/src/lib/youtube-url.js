/** Where a video lives on YouTube itself. */
export function youtubeUrl(video) {
  if (video.is_short) return `https://www.youtube.com/shorts/${video.id}`
  return `https://www.youtube.com/watch?v=${video.id}`
}

/**
 * The embedded player for a video. `playsinline` keeps phones from forcing
 * full screen; `enablejsapi` + `origin` let the page hear the player's errors.
 */
export function embedUrl(videoId, origin) {
  const params = new URLSearchParams({
    autoplay: '1',
    playsinline: '1',
    rel: '0',
    enablejsapi: '1',
    origin,
  })
  return `https://www.youtube.com/embed/${videoId}?${params}`
}
