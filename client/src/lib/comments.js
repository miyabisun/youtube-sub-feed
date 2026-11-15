// Writing comments with the viewer's own short-lived Google token. The target
// video is always the id this code is given — never a "current video" kept by
// someone else.
const API = 'https://www.googleapis.com/youtube/v3'

/** Needed to write comments; reading the subscription list needs less. */
export const COMMENT_SCOPE = 'https://www.googleapis.com/auth/youtube.force-ssl'

/** YouTube's own limit for a comment. */
export const COMMENT_MAX_LENGTH = 10_000

export function canSubmit(text) {
  const trimmed = text.trim()
  return trimmed.length > 0 && trimmed.length <= COMMENT_MAX_LENGTH
}

export function insertCommentRequest(videoId, text) {
  return {
    url: `${API}/commentThreads?part=snippet`,
    init: {
      method: 'POST',
      body: JSON.stringify({
        snippet: { videoId, topLevelComment: { snippet: { textOriginal: text.trim() } } },
      }),
    },
  }
}

export function updateCommentRequest(commentId, text) {
  return {
    url: `${API}/comments?part=snippet`,
    init: {
      method: 'PUT',
      body: JSON.stringify({ id: commentId, snippet: { textOriginal: text.trim() } }),
    },
  }
}

export function deleteCommentRequest(commentId) {
  return {
    url: `${API}/comments?id=${encodeURIComponent(commentId)}`,
    init: { method: 'DELETE' },
  }
}

/**
 * Reduce an inserted thread, or an updated comment, to the item shape that
 * GET /api/videos/{id}/comments returns.
 */
export function postedComment(resource) {
  const comment = resource.snippet?.topLevelComment ?? resource
  const snippet = comment.snippet ?? {}
  return {
    id: comment.id,
    author: snippet.authorDisplayName ?? '',
    author_avatar: snippet.authorProfileImageUrl ?? null,
    text: snippet.textOriginal ?? '',
    like_count: snippet.likeCount ?? 0,
    published_at: snippet.publishedAt ?? null,
  }
}

export class CommentError extends Error {
  constructor(status, reason) {
    super(`${status} ${reason}`)
    this.status = status
    this.reason = reason
  }
}

const REASONS = {
  commentsDisabled: 'この動画はコメントを受け付けていません',
  quotaExceeded: '今日の API の割り当てを使い切りました。明日もう一度送ってください',
  ineligibleAccount: 'YouTube チャンネルの無い Google アカウントではコメントできません',
  insufficientPermissions: 'Google の認可が足りません。もう一度送って認可し直してください',
  commentTextTooLong: 'コメントが長すぎます',
  videoNotFound: '動画が見つかりません',
  commentNotFound: 'コメントが見つかりません',
}

export function commentErrorMessage(error) {
  if (error instanceof CommentError) {
    if (REASONS[error.reason]) return REASONS[error.reason]
    if (error.status === 401) return 'Google の認可が切れました。もう一度送ってください'
    return `送信できませんでした (${error.status} ${error.reason})`
  }
  const message = error?.message ?? ''
  if (message.includes('cancelled') || message.includes('popup_closed')) {
    return 'Google の認可がキャンセルされました'
  }
  if (message.startsWith('GIS ')) return message
  return '送信できませんでした。通信を確かめてください'
}

/**
 * Send one comment request with the viewer's token. Resolves to the response
 * body (null when the API answers without one), rejects with CommentError.
 */
export async function sendComment({ url, init }, { tokens, fetchFn = fetch }) {
  const token = await tokens.get()
  const response = await fetchFn(url, {
    ...init,
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
  })
  const body = await response.json().catch(() => null)
  if (response.ok) return body
  if (response.status === 401) tokens.invalidate()
  throw new CommentError(response.status, body?.error?.errors?.[0]?.reason ?? 'unknown')
}
