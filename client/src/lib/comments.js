// Writing comments as the viewer. The target video is always the id this code
// is given — never a "current video" kept by someone else.
import { API, writeErrorMessage } from './youtube-write.js'

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

const REASONS = {
  commentsDisabled: 'この動画はコメントを受け付けていません',
  ineligibleAccount: 'YouTube チャンネルの無い Google アカウントではコメントできません',
  commentTextTooLong: 'コメントが長すぎます',
  commentNotFound: 'コメントが見つかりません',
}

export function commentErrorMessage(error) {
  return writeErrorMessage(error, REASONS)
}
