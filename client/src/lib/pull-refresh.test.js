import { describe, expect, test } from 'vitest'
import { PULL_MAX, PULL_THRESHOLD, isPull, pullDistance, shouldRefresh } from './pull-refresh.js'
import { lockAxis } from './swipe.js'

describe('isPull', () => {
  test('is undecided while the finger is still inside the slop', () => {
    expect(isPull(0, 0)).toBe(null)
    expect(isPull(4, 4)).toBe(null)
    expect(isPull(-4, -4)).toBe(null)
  })

  test('a downward move is a pull', () => {
    expect(isPull(0, 5)).toBe(true)
    expect(isPull(3, 40)).toBe(true)
    expect(isPull(-20, 21)).toBe(true)
  })

  test('an upward move is a scroll, not a pull', () => {
    expect(isPull(0, -5)).toBe(false)
    expect(isPull(2, -60)).toBe(false)
  })

  test('a move that is more sideways than down belongs to the group swipe', () => {
    expect(isPull(40, 10)).toBe(false)
    expect(isPull(-21, 20)).toBe(false)
  })

  test('a diagonal is never both a pull and a sideways swipe', () => {
    for (const [dx, dy] of [
      [30, 30],
      [-30, 30],
      [31, 30],
      [30, 31],
      [5, 5],
      [-80, 79],
    ]) {
      const swipe = lockAxis(dx, dy) === 'horizontal'
      expect([dx, dy, swipe && isPull(dx, dy)]).toEqual([dx, dy, false])
      expect([dx, dy, swipe || isPull(dx, dy)]).toEqual([dx, dy, true])
    }
  })
})

describe('pullDistance', () => {
  test('follows the finger at half its travel', () => {
    expect(pullDistance(0)).toBe(0)
    expect(pullDistance(40)).toBe(20)
    expect(pullDistance(100)).toBe(50)
  })

  test('never goes above the list for a finger that moved back up', () => {
    expect(pullDistance(-30)).toBe(0)
  })

  test('stops growing at the maximum', () => {
    expect(pullDistance(PULL_MAX * 2)).toBe(PULL_MAX)
    expect(pullDistance(5000)).toBe(PULL_MAX)
  })
})

describe('shouldRefresh', () => {
  test('refreshes only from the threshold on', () => {
    expect(shouldRefresh(0)).toBe(false)
    expect(shouldRefresh(PULL_THRESHOLD - 1)).toBe(false)
    expect(shouldRefresh(PULL_THRESHOLD)).toBe(true)
    expect(shouldRefresh(PULL_MAX)).toBe(true)
  })

  test('the threshold is within reach of a pull', () => {
    expect(PULL_THRESHOLD).toBeLessThan(PULL_MAX)
    expect(shouldRefresh(pullDistance(PULL_THRESHOLD * 2))).toBe(true)
    expect(shouldRefresh(pullDistance(PULL_THRESHOLD * 2 - 2))).toBe(false)
  })
})
