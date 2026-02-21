let quotaExceeded = false
let quotaResetTime: number | null = null

export class QuotaExceededError extends Error {
  constructor() {
    super('YouTube API quota exceeded')
    this.name = 'QuotaExceededError'
  }
}

export function isQuotaExceeded(): boolean {
  if (!quotaExceeded) return false
  if (quotaResetTime && Date.now() >= quotaResetTime) {
    resetQuota()
    return false
  }
  return true
}

export function setQuotaExceeded(): void {
  quotaExceeded = true
  quotaResetTime = getNextPacificMidnight()
  console.log(`[quota] Quota exceeded. Will reset at ${new Date(quotaResetTime).toISOString()}`)
}

export function resetQuota(): void {
  quotaExceeded = false
  quotaResetTime = null
  console.log('[quota] Quota reset')
}

export function getNextPacificMidnight(): number {
  // Pacific Time is UTC-8 (PST) or UTC-7 (PDT)
  // Use America/Los_Angeles to handle DST
  const now = new Date()
  const pacificNow = new Date(now.toLocaleString('en-US', { timeZone: 'America/Los_Angeles' }))
  const tomorrow = new Date(pacificNow)
  tomorrow.setDate(tomorrow.getDate() + 1)
  tomorrow.setHours(0, 0, 0, 0)

  // Convert back to UTC
  const diff = now.getTime() - pacificNow.getTime()
  return tomorrow.getTime() + diff
}

export function getQuotaResetTime(): number | null {
  return quotaResetTime
}
