import { describe, test, expect } from 'vitest'
import { youtubeUrl } from './youtube-url.js'

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
