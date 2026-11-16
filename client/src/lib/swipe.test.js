import { describe, expect, test } from 'vitest'
import { lockAxis } from './swipe.js'

describe('lockAxis', () => {
  test('stays undecided inside the 5px slop', () => {
    expect(lockAxis(0, 0)).toBe(null)
    expect(lockAxis(4, -4)).toBe(null)
  })

  test('locks to the axis the finger moved further along', () => {
    expect(lockAxis(5, 0)).toBe('horizontal')
    expect(lockAxis(-30, 10)).toBe('horizontal')
    expect(lockAxis(0, 5)).toBe('vertical')
    expect(lockAxis(10, -30)).toBe('vertical')
  })

  test('an exact diagonal is vertical, so it scrolls instead of swiping', () => {
    expect(lockAxis(20, 20)).toBe('vertical')
    expect(lockAxis(-20, 20)).toBe('vertical')
  })
})
