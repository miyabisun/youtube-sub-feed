/** Where a video lives on YouTube itself. */
export function youtubeUrl(video) {
  if (video.is_short) return `https://www.youtube.com/shorts/${video.id}`
  return `https://www.youtube.com/watch?v=${video.id}`
}
