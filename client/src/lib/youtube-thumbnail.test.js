import { describe, test, expect } from 'vitest'
import { videoThumbnail } from './youtube-thumbnail.js'

describe('videoThumbnail', () => {
  test('returns null when videoId is null', () => {
    expect(videoThumbnail(null)).toBe(null)
  })

  test('returns null when videoId is undefined', () => {
    expect(videoThumbnail(undefined)).toBe(null)
  })

  test('returns null when videoId is an empty string', () => {
    expect(videoThumbnail('')).toBe(null)
  })

  test('builds an hqdefault URL by default', () => {
    expect(videoThumbnail('abc123')).toBe('https://i.ytimg.com/vi/abc123/hqdefault.jpg')
  })

  test('honors an explicit quality parameter', () => {
    expect(videoThumbnail('abc123', 'mqdefault')).toBe(
      'https://i.ytimg.com/vi/abc123/mqdefault.jpg',
    )
  })

  test('honors maxresdefault quality', () => {
    expect(videoThumbnail('xyz', 'maxresdefault')).toBe(
      'https://i.ytimg.com/vi/xyz/maxresdefault.jpg',
    )
  })
})
