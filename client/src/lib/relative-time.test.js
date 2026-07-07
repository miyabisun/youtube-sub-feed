import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest'
import { relativeTime } from './relative-time.js'

describe('relativeTime', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2025-06-15T12:00:00Z'))
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  test('seconds ago shows たった今', () => {
    expect(relativeTime('2025-06-15T11:59:30Z')).toBe('たった今')
  })

  test('minutes ago', () => {
    expect(relativeTime('2025-06-15T11:30:00Z')).toBe('30分前')
  })

  test('1 minute ago', () => {
    expect(relativeTime('2025-06-15T11:59:00Z')).toBe('1分前')
  })

  test('hours ago', () => {
    expect(relativeTime('2025-06-15T09:00:00Z')).toBe('3時間前')
  })

  test('days ago', () => {
    expect(relativeTime('2025-06-10T12:00:00Z')).toBe('5日前')
  })

  test('months ago', () => {
    expect(relativeTime('2025-03-15T12:00:00Z')).toBe('3ヶ月前')
  })

  test('years ago', () => {
    expect(relativeTime('2023-06-15T12:00:00Z')).toBe('2年前')
  })

  test('boundary: 59 minutes shows minutes', () => {
    expect(relativeTime('2025-06-15T11:01:00Z')).toBe('59分前')
  })

  test('boundary: 23 hours shows hours', () => {
    expect(relativeTime('2025-06-14T13:00:00Z')).toBe('23時間前')
  })

  test('boundary: 29 days shows days', () => {
    expect(relativeTime('2025-05-17T12:00:00Z')).toBe('29日前')
  })

  // Lower boundaries of each unit: exactly at the threshold the next-larger unit
  // takes over.
  test('boundary: exactly 60 minutes shows 1 hour', () => {
    expect(relativeTime('2025-06-15T11:00:00Z')).toBe('1時間前')
  })

  test('boundary: exactly 24 hours shows 1 day', () => {
    expect(relativeTime('2025-06-14T12:00:00Z')).toBe('1日前')
  })

  test('boundary: exactly 30 days shows 1 month', () => {
    expect(relativeTime('2025-05-16T12:00:00Z')).toBe('1ヶ月前')
  })

  test('boundary: exactly 365 days shows 1 year', () => {
    expect(relativeTime('2024-06-15T12:00:00Z')).toBe('1年前')
  })

  // A timestamp in the future yields a negative diff; every unit floors below
  // zero, so it collapses to the "たった今" fallback rather than a "-N前".
  test('future timestamp shows たった今', () => {
    expect(relativeTime('2025-06-15T13:00:00Z')).toBe('たった今')
  })

  test('null returns empty', () => {
    expect(relativeTime(null)).toBe('')
  })

  test('undefined returns empty', () => {
    expect(relativeTime(undefined)).toBe('')
  })

  test('empty string returns empty', () => {
    expect(relativeTime('')).toBe('')
  })
})
