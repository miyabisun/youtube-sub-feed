import { lockAxis } from './swipe.js'

// How far the strip must be open for a release to refresh, and how far it opens at most.
export const PULL_THRESHOLD = 64
export const PULL_MAX = 96
// How far it stays open while the refresh runs.
export const PULL_HOLD = 48

/**
 * Whether a touch that moved by (dx, dy) is a pull: `null` while it is still
 * undecided, then `true` for a move down. Sideways moves are left to the
 * swipe gestures, which lock on the same rule, so one touch is never both.
 */
export function isPull(dx, dy) {
  const axis = lockAxis(dx, dy)
  if (!axis) return null
  return axis === 'vertical' && dy > 0
}

/** How far the strip opens for a finger that has moved `dy` px down. */
export function pullDistance(dy) {
  return Math.min(PULL_MAX, Math.max(0, dy) / 2)
}

/** Whether letting go with the strip open this far refreshes. */
export function shouldRefresh(distance) {
  return distance >= PULL_THRESHOLD
}

function scrollParent(node) {
  for (let el = node.parentElement; el; el = el.parentElement) {
    const { overflowY } = getComputedStyle(el)
    if (overflowY === 'auto' || overflowY === 'scroll') return el
  }
  return document.documentElement
}

/**
 * Pull-to-refresh for a touch screen. A touch that starts with the list at
 * its top and moves down is taken from the browser — so its own
 * pull-to-refresh never runs beside this one — and reported through
 * `onPull(distance)`; letting go past the threshold calls `onRefresh()`.
 * While `busy()` is true the touch is still taken, and does nothing.
 *
 * The page keeps the browser's overscroll inside the scroller as well
 * (`overscroll-behavior`), for a move the browser will not give up.
 */
export function pullToRefresh(node, { onPull, onRefresh, busy = () => false }) {
  const scroller = scrollParent(node)

  let startX, startY
  let tracking = false
  let pulling = false
  let distance = 0

  // Nothing between the touched element and the scroller is scrolled down.
  function atTop(target) {
    for (let el = target; el; el = el.parentElement) {
      if (el.scrollTop > 0) return false
      if (el === scroller) break
    }
    return true
  }

  function stop() {
    tracking = false
    if (!pulling) return
    pulling = false
    distance = 0
    onPull(0)
  }

  function onStart(e) {
    stop()
    if (e.touches.length !== 1 || !atTop(e.target)) return
    tracking = true
    startX = e.touches[0].clientX
    startY = e.touches[0].clientY
  }

  function onMove(e) {
    if (!tracking) return
    if (e.touches.length !== 1) return stop()
    const dx = e.touches[0].clientX - startX
    const dy = e.touches[0].clientY - startY

    if (!pulling) {
      const pull = isPull(dx, dy)
      if (pull === null) return
      if (!pull) return stop()
      pulling = true
    }
    if (e.cancelable) e.preventDefault()
    if (busy()) return
    distance = pullDistance(dy)
    onPull(distance)
  }

  function onEnd() {
    const refresh = pulling && shouldRefresh(distance)
    stop()
    if (refresh) onRefresh()
  }

  scroller.addEventListener('touchstart', onStart, { passive: true })
  scroller.addEventListener('touchmove', onMove, { passive: false })
  scroller.addEventListener('touchend', onEnd, { passive: true })
  scroller.addEventListener('touchcancel', stop, { passive: true })

  return {
    destroy() {
      scroller.removeEventListener('touchstart', onStart)
      scroller.removeEventListener('touchmove', onMove)
      scroller.removeEventListener('touchend', onEnd)
      scroller.removeEventListener('touchcancel', stop)
    },
  }
}
