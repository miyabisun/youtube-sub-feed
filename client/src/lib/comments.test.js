import { describe, test, expect } from 'vitest'
import {
  COMMENT_MAX_LENGTH,
  canSubmit,
  insertCommentRequest,
  updateCommentRequest,
  deleteCommentRequest,
  postedComment,
  CommentError,
  commentErrorMessage,
  sendComment,
} from './comments.js'

describe('canSubmit', () => {
  test('requires something other than whitespace', () => {
    expect(canSubmit('')).toBe(false)
    expect(canSubmit('  \n ')).toBe(false)
    expect(canSubmit('よかった')).toBe(true)
  })

  test('stops at the YouTube length limit', () => {
    expect(canSubmit('あ'.repeat(COMMENT_MAX_LENGTH))).toBe(true)
    expect(canSubmit('あ'.repeat(COMMENT_MAX_LENGTH + 1))).toBe(false)
  })
})

describe('insertCommentRequest', () => {
  test('addresses the comment to the given video', () => {
    const { url, init } = insertCommentRequest('stw7lYY3W3I', '最高でした')
    expect(url).toBe('https://www.googleapis.com/youtube/v3/commentThreads?part=snippet')
    expect(init.method).toBe('POST')
    expect(JSON.parse(init.body)).toEqual({
      snippet: {
        videoId: 'stw7lYY3W3I',
        topLevelComment: { snippet: { textOriginal: '最高でした' } },
      },
    })
  })

  test('keeps line breaks and inner spacing, trimming only the ends', () => {
    const { init } = insertCommentRequest('stw7lYY3W3I', '\n 1行目\n\n  2行目 \n')
    expect(JSON.parse(init.body).snippet.topLevelComment.snippet.textOriginal).toBe(
      '1行目\n\n  2行目',
    )
  })
})

describe('updateCommentRequest', () => {
  test('replaces the text of one comment', () => {
    const { url, init } = updateCommentRequest('c1', '直しました')
    expect(url).toBe('https://www.googleapis.com/youtube/v3/comments?part=snippet')
    expect(init.method).toBe('PUT')
    expect(JSON.parse(init.body)).toEqual({ id: 'c1', snippet: { textOriginal: '直しました' } })
  })
})

describe('deleteCommentRequest', () => {
  test('deletes one comment by id', () => {
    const { url, init } = deleteCommentRequest('Ugx-1.a b')
    expect(url).toBe('https://www.googleapis.com/youtube/v3/comments?id=Ugx-1.a%20b')
    expect(init.method).toBe('DELETE')
  })
})

describe('postedComment', () => {
  test('reduces an inserted thread to the shape the list renders', () => {
    const thread = {
      id: 'thread1',
      snippet: {
        topLevelComment: {
          id: 'c1',
          snippet: {
            authorDisplayName: '@me',
            authorProfileImageUrl: 'https://yt3.ggpht.com/me',
            textOriginal: '最高でした',
            likeCount: 0,
            publishedAt: '2026-11-15T00:00:00Z',
          },
        },
      },
    }
    expect(postedComment(thread)).toEqual({
      id: 'c1',
      author: '@me',
      author_avatar: 'https://yt3.ggpht.com/me',
      text: '最高でした',
      like_count: 0,
      published_at: '2026-11-15T00:00:00Z',
    })
  })

  test('reads an updated comment resource too', () => {
    const comment = { id: 'c1', snippet: { textOriginal: '直しました' } }
    expect(postedComment(comment)).toMatchObject({ id: 'c1', text: '直しました' })
  })
})

describe('commentErrorMessage', () => {
  const message = (status, reason) => commentErrorMessage(new CommentError(status, reason))

  test('explains disabled comments', () => {
    expect(message(403, 'commentsDisabled')).toBe('この動画はコメントを受け付けていません')
  })

  test('explains an exhausted quota', () => {
    expect(message(403, 'quotaExceeded')).toMatch(/割り当て/)
  })

  test('asks to authorize again when the token is rejected', () => {
    expect(message(401, 'authError')).toMatch(/認可/)
    expect(message(403, 'insufficientPermissions')).toMatch(/認可/)
  })

  test('explains an account without a channel', () => {
    expect(message(403, 'ineligibleAccount')).toMatch(/チャンネル/)
  })

  test('explains a video that is gone', () => {
    expect(message(404, 'videoNotFound')).toBe('動画が見つかりません')
  })

  test('explains a cancelled Google authorization', () => {
    expect(commentErrorMessage(new Error('popup_closed'))).toBe(
      'Google の認可がキャンセルされました',
    )
  })

  test('keeps the reason visible for anything unexpected', () => {
    expect(message(400, 'processingFailure')).toBe('送信できませんでした (400 processingFailure)')
    expect(commentErrorMessage(new TypeError('Failed to fetch'))).toBe(
      '送信できませんでした。通信を確かめてください',
    )
  })
})

describe('sendComment', () => {
  const request = { url: 'https://example.test/x', init: { method: 'POST', body: '{}' } }

  function tokens(values) {
    const calls = { get: 0, invalidate: 0 }
    return {
      calls,
      get: async () => values[calls.get++],
      invalidate: () => calls.invalidate++,
    }
  }

  test('sends with the bearer token and returns the parsed body', async () => {
    const seen = []
    const fetchFn = async (url, init) => {
      seen.push({ url, init })
      return { ok: true, status: 200, json: async () => ({ id: 'c1' }) }
    }
    const body = await sendComment(request, { tokens: tokens(['T1']), fetchFn })
    expect(body).toEqual({ id: 'c1' })
    expect(seen[0].url).toBe('https://example.test/x')
    expect(seen[0].init.headers.Authorization).toBe('Bearer T1')
    expect(seen[0].init.headers['Content-Type']).toBe('application/json')
    expect(seen[0].init.body).toBe('{}')
  })

  test('returns null for an empty success body (delete)', async () => {
    const fetchFn = async () => ({ ok: true, status: 204, json: async () => Promise.reject() })
    expect(await sendComment(request, { tokens: tokens(['T1']), fetchFn })).toBeNull()
  })

  test('throws the API reason and drops a rejected token', async () => {
    const store = tokens(['T1'])
    const fetchFn = async () => ({
      ok: false,
      status: 401,
      json: async () => ({ error: { errors: [{ reason: 'authError' }] } }),
    })
    await expect(sendComment(request, { tokens: store, fetchFn })).rejects.toMatchObject({
      status: 401,
      reason: 'authError',
    })
    expect(store.calls.invalidate).toBe(1)
  })

  test('keeps the token on failures that are not about authorization', async () => {
    const store = tokens(['T1'])
    const fetchFn = async () => ({
      ok: false,
      status: 403,
      json: async () => ({ error: { errors: [{ reason: 'commentsDisabled' }] } }),
    })
    await expect(sendComment(request, { tokens: store, fetchFn })).rejects.toMatchObject({
      status: 403,
      reason: 'commentsDisabled',
    })
    expect(store.calls.invalidate).toBe(0)
  })
})
