import { describe, test, expect } from 'vitest'
import {
  isLiked,
  rateVideoRequest,
  ratingErrorMessage,
  ratingRequest,
  ratingsDisabled,
} from './rating.js'
import { WriteError } from './youtube-write.js'

describe('rateVideoRequest', () => {
  test('likes the given video', () => {
    const { url, init } = rateVideoRequest('stw7lYY3W3I', true)
    expect(url).toBe('https://www.googleapis.com/youtube/v3/videos/rate?id=stw7lYY3W3I&rating=like')
    expect(init).toEqual({ method: 'POST' })
  })

  test('takes the like off again', () => {
    expect(rateVideoRequest('stw7lYY3W3I', false).url).toBe(
      'https://www.googleapis.com/youtube/v3/videos/rate?id=stw7lYY3W3I&rating=none',
    )
  })

  test('escapes the id', () => {
    expect(rateVideoRequest('a b&c', true).url).toContain('id=a%20b%26c&rating=like')
  })
})

describe('ratingRequest', () => {
  test('asks for the viewer rating of one video', () => {
    const { url, init } = ratingRequest('stw7lYY3W3I')
    expect(url).toBe('https://www.googleapis.com/youtube/v3/videos/getRating?id=stw7lYY3W3I')
    expect(init).toEqual({ method: 'GET' })
  })
})

describe('isLiked', () => {
  const body = (rating, videoId = 'v1') => ({ items: [{ videoId, rating }] })

  test('is true only for a like on that video', () => {
    expect(isLiked(body('like'), 'v1')).toBe(true)
    expect(isLiked(body('none'), 'v1')).toBe(false)
    expect(isLiked(body('dislike'), 'v1')).toBe(false)
    expect(isLiked(body('like', 'other'), 'v1')).toBe(false)
  })

  test('is false when the answer is empty', () => {
    expect(isLiked(null, 'v1')).toBe(false)
    expect(isLiked({}, 'v1')).toBe(false)
  })
})

describe('ratingsDisabled', () => {
  test('recognises a video whose owner turned ratings off', () => {
    expect(ratingsDisabled(new WriteError(403, 'videoRatingDisabled'))).toBe(true)
    expect(ratingsDisabled(new WriteError(403, 'quotaExceeded'))).toBe(false)
    expect(ratingsDisabled(new Error('popup_closed'))).toBe(false)
  })
})

describe('ratingErrorMessage', () => {
  const message = (status, reason) => ratingErrorMessage(new WriteError(status, reason))

  test('explains a video that cannot be rated', () => {
    expect(message(403, 'forbidden')).toBe('この動画は評価できません')
  })

  test('shares the wording for quota, authorization and the unexpected', () => {
    expect(message(403, 'quotaExceeded')).toMatch(/割り当て/)
    expect(message(401, 'authError')).toMatch(/認可/)
    expect(message(400, 'invalidRating')).toBe('送信できませんでした (400 invalidRating)')
    expect(ratingErrorMessage(new Error('popup_closed'))).toBe(
      'Google の認可がキャンセルされました',
    )
  })
})
