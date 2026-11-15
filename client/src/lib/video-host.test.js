import { describe, test, expect } from 'vitest'
import { hasVideoHost, openVideoMessage } from './video-host.js'

describe('hasVideoHost', () => {
  const root = (dataset) => ({ dataset })

  test('is true only when the host marked the page', () => {
    expect(hasVideoHost(root({ videoHost: '1' }))).toBe(true)
    expect(hasVideoHost(root({}))).toBe(false)
    expect(hasVideoHost(root({ videoHost: '' }))).toBe(false)
  })

  test('is false where there is no document at all', () => {
    expect(hasVideoHost(undefined)).toBe(false)
  })
})

describe('openVideoMessage', () => {
  test('names the video and where it lives on YouTube', () => {
    expect(openVideoMessage({ id: 'stw7lYY3W3I', is_short: 0 })).toEqual({
      source: 'youtube-sub-feed',
      type: 'open-video',
      id: 'stw7lYY3W3I',
      url: 'https://www.youtube.com/watch?v=stw7lYY3W3I',
    })
  })

  test('sends a Short to its Shorts address', () => {
    expect(openVideoMessage({ id: 'abc', is_short: 1 }).url).toBe(
      'https://www.youtube.com/shorts/abc',
    )
  })
})
