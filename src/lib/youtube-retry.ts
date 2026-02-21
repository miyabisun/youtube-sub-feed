import { QuotaExceededError, setQuotaExceeded } from './quota-manager.js'

const MAX_RETRIES = 3
const BACKOFF_BASE_MS = 1000

export async function withRetry<T>(fn: () => Promise<T>): Promise<T> {
  for (let attempt = 0; attempt < MAX_RETRIES; attempt++) {
    try {
      return await fn()
    } catch (e: any) {
      if (e instanceof QuotaExceededError) throw e

      // Check if it's a quota exceeded response
      if (e?.status === 403 && e?.reason === 'quotaExceeded') {
        setQuotaExceeded()
        throw new QuotaExceededError()
      }

      if (attempt < MAX_RETRIES - 1) {
        const delay = BACKOFF_BASE_MS * (attempt + 1)
        await new Promise((r) => setTimeout(r, delay))
      } else {
        throw e
      }
    }
  }
  throw new Error('Unreachable')
}
