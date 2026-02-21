import { describe, test, expect, beforeEach } from 'bun:test'
import { isQuotaExceeded, setQuotaExceeded, resetQuota, getNextPacificMidnight, getQuotaResetTime } from '../quota-manager.js'

beforeEach(() => {
  resetQuota()
})

describe('quota-manager', () => {
  test('initially not exceeded', () => {
    expect(isQuotaExceeded()).toBe(false)
  })

  test('set exceeded returns true', () => {
    setQuotaExceeded()
    expect(isQuotaExceeded()).toBe(true)
  })

  test('reset clears exceeded state', () => {
    setQuotaExceeded()
    resetQuota()
    expect(isQuotaExceeded()).toBe(false)
  })

  test('sets reset time when exceeded', () => {
    setQuotaExceeded()
    const resetTime = getQuotaResetTime()
    expect(resetTime).not.toBeNull()
    expect(resetTime!).toBeGreaterThan(Date.now())
  })

  test('getNextPacificMidnight returns a future timestamp', () => {
    const midnight = getNextPacificMidnight()
    expect(midnight).toBeGreaterThan(Date.now())
    // Should be within ~25 hours from now
    expect(midnight - Date.now()).toBeLessThan(25 * 60 * 60 * 1000)
  })

  test('reset time is cleared after reset', () => {
    setQuotaExceeded()
    resetQuota()
    expect(getQuotaResetTime()).toBeNull()
  })
})
