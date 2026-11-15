// The viewer's like on a video. The target is always the id this code is
// given, like the comments.
import { API, WriteError, writeErrorMessage } from './youtube-write.js'

export function rateVideoRequest(videoId, liked) {
  const rating = liked ? 'like' : 'none'
  return {
    url: `${API}/videos/rate?id=${encodeURIComponent(videoId)}&rating=${rating}`,
    init: { method: 'POST' },
  }
}

export function ratingRequest(videoId) {
  return {
    url: `${API}/videos/getRating?id=${encodeURIComponent(videoId)}`,
    init: { method: 'GET' },
  }
}

/** Read the answer to ratingRequest. */
export function isLiked(body, videoId) {
  return (body?.items ?? []).some((item) => item.videoId === videoId && item.rating === 'like')
}

/** The owner turned ratings off: a property of the video, not a failure. */
export function ratingsDisabled(error) {
  return error instanceof WriteError && error.reason === 'videoRatingDisabled'
}

const REASONS = {
  forbidden: 'この動画は評価できません',
  emailNotVerified: 'Google アカウントのメールアドレスを確認してから評価してください',
  videoPurchaseRequired: 'この動画はレンタルした人だけが評価できます',
}

export function ratingErrorMessage(error) {
  return writeErrorMessage(error, REASONS)
}
