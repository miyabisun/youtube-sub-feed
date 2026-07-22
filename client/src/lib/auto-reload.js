export const AUTO_RELOAD_INTERVAL_MS = 60_000

/**
 * Reload periodically while the page is visible.
 *
 * The initial load remains the caller's responsibility. Returning to a
 * visible tab reloads immediately, then starts a fresh interval.
 */
export function startAutoReload(
  reload,
  { documentRef = document, timers = globalThis, intervalMs = AUTO_RELOAD_INTERVAL_MS } = {},
) {
  let intervalId = null

  function stopInterval() {
    if (intervalId === null) return
    timers.clearInterval(intervalId)
    intervalId = null
  }

  function startInterval() {
    stopInterval()
    if (documentRef.visibilityState !== 'visible') return
    intervalId = timers.setInterval(() => reload(), intervalMs)
  }

  function handleVisibilityChange() {
    if (documentRef.visibilityState !== 'visible') {
      stopInterval()
      return
    }
    reload()
    startInterval()
  }

  documentRef.addEventListener('visibilitychange', handleVisibilityChange)
  startInterval()

  return () => {
    stopInterval()
    documentRef.removeEventListener('visibilitychange', handleVisibilityChange)
  }
}
