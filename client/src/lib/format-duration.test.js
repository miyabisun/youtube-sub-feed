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

  // YouTube returns a date part for durations >= 24h (e.g. long live archives).
  // 1 day is folded into hours (1D = 24h).
  test('day part folds into hours (P1DT2H3M4S -> 26:03:04)', () => {
    expect(formatDuration('P1DT2H3M4S')).toBe('26:03:04')
  })

  test('multiple days fold into hours (P2DT0H0M0S -> 48:00:00)', () => {
    expect(formatDuration('P2DT0H0M0S')).toBe('48:00:00')
  })

  test('day part with only minutes and seconds', () => {
    // 1 day = 24h, no explicit hours component.
    expect(formatDuration('P1DT30M15S')).toBe('24:30:15')
  })
})
