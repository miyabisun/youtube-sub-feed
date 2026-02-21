import { describe, test, expect } from 'bun:test'

// Inline pure function to test (same logic as client/src/lib/relative-time.js)
function relativeTime(dateStr: string | null): string {
  if (!dateStr) return ''
  const now = Date.now()
  const then = new Date(dateStr).getTime()
  const diff = now - then

  const seconds = Math.floor(diff / 1000)
  const minutes = Math.floor(seconds / 60)
  const hours = Math.floor(minutes / 60)
  const days = Math.floor(hours / 24)
  const months = Math.floor(days / 30)
  const years = Math.floor(days / 365)

  if (years > 0) return `${years}年前`
  if (months > 0) return `${months}ヶ月前`
  if (days > 0) return `${days}日前`
  if (hours > 0) return `${hours}時間前`
  if (minutes > 0) return `${minutes}分前`
  return 'たった今'
}

describe('relativeTime', () => {
  test('returns empty string for null', () => {
    expect(relativeTime(null)).toBe('')
  })

  test('returns "たった今" for recent time', () => {
    const now = new Date().toISOString()
    expect(relativeTime(now)).toBe('たった今')
  })

  test('returns minutes ago', () => {
    const date = new Date(Date.now() - 5 * 60 * 1000).toISOString()
    expect(relativeTime(date)).toBe('5分前')
  })

  test('returns hours ago', () => {
    const date = new Date(Date.now() - 3 * 60 * 60 * 1000).toISOString()
    expect(relativeTime(date)).toBe('3時間前')
  })

  test('returns days ago', () => {
    const date = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString()
    expect(relativeTime(date)).toBe('7日前')
  })

  test('returns months ago', () => {
    const date = new Date(Date.now() - 90 * 24 * 60 * 60 * 1000).toISOString()
    expect(relativeTime(date)).toBe('3ヶ月前')
  })

  test('returns years ago', () => {
    const date = new Date(Date.now() - 400 * 24 * 60 * 60 * 1000).toISOString()
    expect(relativeTime(date)).toBe('1年前')
  })
})
