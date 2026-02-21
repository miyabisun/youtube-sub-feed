import { describe, test, expect } from 'bun:test'
import { parseISODuration, formatDuration, isShortDuration } from '../duration.js'

describe('parseISODuration', () => {
  test('parses hours, minutes, and seconds', () => {
    expect(parseISODuration('PT1H2M3S')).toBe(3723)
  })

  test('parses minutes and seconds only', () => {
    expect(parseISODuration('PT5M30S')).toBe(330)
  })

  test('parses seconds only', () => {
    expect(parseISODuration('PT45S')).toBe(45)
  })

  test('parses minutes only', () => {
    expect(parseISODuration('PT10M')).toBe(600)
  })

  test('parses hours only', () => {
    expect(parseISODuration('PT2H')).toBe(7200)
  })

  test('returns 0 for empty duration', () => {
    expect(parseISODuration('P0D')).toBe(0)
    expect(parseISODuration('PT0S')).toBe(0)
  })

  test('returns 0 for invalid input', () => {
    expect(parseISODuration('')).toBe(0)
    expect(parseISODuration('invalid')).toBe(0)
  })

  test('handles boundary: exactly 60 seconds', () => {
    expect(parseISODuration('PT1M')).toBe(60)
    expect(parseISODuration('PT60S')).toBe(60)
  })
})

describe('formatDuration', () => {
  test('formats hours, minutes, seconds', () => {
    expect(formatDuration('PT1H2M3S')).toBe('1:02:03')
  })

  test('formats minutes and seconds', () => {
    expect(formatDuration('PT5M30S')).toBe('5:30')
  })

  test('formats seconds only with leading zero', () => {
    expect(formatDuration('PT45S')).toBe('0:45')
  })

  test('formats zero duration', () => {
    expect(formatDuration('PT0S')).toBe('0:00')
  })

  test('formats large duration', () => {
    expect(formatDuration('PT12H0M0S')).toBe('12:00:00')
  })

  test('returns empty string for invalid', () => {
    expect(formatDuration('')).toBe('0:00')
  })
})

describe('isShortDuration', () => {
  test('returns true for 60 seconds or less', () => {
    expect(isShortDuration('PT45S')).toBe(true)
    expect(isShortDuration('PT60S')).toBe(true)
    expect(isShortDuration('PT1M')).toBe(true)
  })

  test('returns false for more than 60 seconds', () => {
    expect(isShortDuration('PT61S')).toBe(false)
    expect(isShortDuration('PT1M1S')).toBe(false)
    expect(isShortDuration('PT5M')).toBe(false)
  })

  test('returns false for zero duration', () => {
    expect(isShortDuration('PT0S')).toBe(false)
  })
})
