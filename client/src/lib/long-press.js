export const LONG_PRESS_MS = 500

/**
 * Track a touch long-press while allowing normal taps and scroll gestures.
 * The payload supplied at pointer-down is returned to the callback.
 */
export function createLongPress(onLongPress, delay = LONG_PRESS_MS) {
  let timer = null
  let startX = 0
  let startY = 0
  let longPressed = false

  function stopTimer() {
    if (timer !== null) clearTimeout(timer)
    timer = null
  }

  function abort() {
    stopTimer()
    longPressed = false
  }

  function pointerDown(event, payload) {
    if (event.pointerType !== 'touch') return
    abort()
    longPressed = false
    startX = event.clientX
    startY = event.clientY
    timer = setTimeout(() => {
      timer = null
      longPressed = true
      onLongPress(payload)
    }, delay)
  }

  function pointerMove(event) {
    if (timer === null) return
    if (Math.hypot(event.clientX - startX, event.clientY - startY) > 10) abort()
  }

  function consumeClick() {
    if (!longPressed) return false
    longPressed = false
    return true
  }

  return { pointerDown, pointerMove, pointerUp: stopTimer, abort, consumeClick }
}
