import { describe, test, expect } from 'vitest'
import { formatDuration } from './format-duration.js'

describe('formatDuration', () => {
  test('hours, minutes, seconds', () => {
    expect(formatDuration('PT1H2M3S')).toBe('1:02:03')
  })

  test('minutes and seconds', () => {
    expect(formatDuration('PT10M30S')).toBe('10:30')
  })

  test('seconds only', () => {
    expect(formatDuration('PT45S')).toBe('0:45')
  })

  test('minutes only', () => {
    expect(formatDuration('PT5M')).toBe('5:00')
  })

  test('hours only', () => {
    expect(formatDuration('PT2H')).toBe('2:00:00')
  })

  test('hours and minutes', () => {
    expect(formatDuration('PT1H30M')).toBe('1:30:00')
  })

  test('zero duration PT0S', () => {
    expect(formatDuration('PT0S')).toBe('0:00')
  })

  test('long duration over 12h', () => {
    expect(formatDuration('PT12H34M56S')).toBe('12:34:56')
  })

  test('pads seconds with leading zero', () => {
    expect(formatDuration('PT3M5S')).toBe('3:05')
  })

  test('pads minutes with leading zero when hours present', () => {
    expect(formatDuration('PT1H5M3S')).toBe('1:05:03')
  })

  test('null returns empty', () => {
    expect(formatDuration(null)).toBe('')
  })

  test('undefined returns empty', () => {
    expect(formatDuration(undefined)).toBe('')
  })

  test('empty string returns empty', () => {
    expect(formatDuration('')).toBe('')
  })

  test('invalid format returns empty', () => {
    expect(formatDuration('invalid')).toBe('')
  })
})
