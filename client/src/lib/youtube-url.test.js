import { describe, test, expect } from 'vitest'
import { youtubeUrl, embedUrl } from './youtube-url.js'

describe('youtubeUrl', () => {
  test('links a regular video to its watch page', () => {
    expect(youtubeUrl({ id: 'stw7lYY3W3I' })).toBe('https://www.youtube.com/watch?v=stw7lYY3W3I')
  })

  test('links a Short to the Shorts player', () => {
    expect(youtubeUrl({ id: 'stw7lYY3W3I', is_short: 1 })).toBe(
      'https://www.youtube.com/shorts/stw7lYY3W3I',
    )
  })
})

describe('embedUrl', () => {
  test('builds an inline, script-controllable player for the page origin', () => {
    const url = new URL(embedUrl('stw7lYY3W3I', 'https://feed.example.test'))
    expect(url.origin + url.pathname).toBe('https://www.youtube.com/embed/stw7lYY3W3I')
    expect(Object.fromEntries(url.searchParams)).toEqual({
      autoplay: '1',
      playsinline: '1',
      rel: '0',
      enablejsapi: '1',
      origin: 'https://feed.example.test',
    })
  })
})
