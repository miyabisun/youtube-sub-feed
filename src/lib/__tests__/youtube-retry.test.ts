import { describe, test, expect } from 'bun:test'
import { withRetry } from '../youtube-retry.js'
import { QuotaExceededError } from '../quota-manager.js'

describe('withRetry', () => {
  test('returns result on first success', async () => {
    const result = await withRetry(async () => 'ok')
    expect(result).toBe('ok')
  })

  test('retries on failure and succeeds', async () => {
    let attempts = 0
    const result = await withRetry(async () => {
      attempts++
      if (attempts < 3) throw new Error('transient')
      return 'recovered'
    })
    expect(result).toBe('recovered')
    expect(attempts).toBe(3)
  })

  test('throws after max retries', async () => {
    let attempts = 0
    await expect(
      withRetry(async () => {
        attempts++
        throw new Error('persistent')
      })
    ).rejects.toThrow('persistent')
    expect(attempts).toBe(3)
  })

  test('immediately throws QuotaExceededError without retry', async () => {
    let attempts = 0
    await expect(
      withRetry(async () => {
        attempts++
        throw new QuotaExceededError()
      })
    ).rejects.toThrow(QuotaExceededError)
    expect(attempts).toBe(1)
  })

  test('detects quota exceeded from API error and throws', async () => {
    let attempts = 0
    await expect(
      withRetry(async () => {
        attempts++
        const err: any = new Error('quota')
        err.status = 403
        err.reason = 'quotaExceeded'
        throw err
      })
    ).rejects.toThrow(QuotaExceededError)
    expect(attempts).toBe(1)
  })
})
