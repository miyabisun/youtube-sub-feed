import { describe, expect, it, vi } from 'vitest'
import { AUTO_RELOAD_INTERVAL_MS, startAutoReload } from './auto-reload.js'

function harness(initialVisibility = 'visible') {
  const listeners = new Map()
  const intervals = new Map()
  let nextIntervalId = 1

  const documentRef = {
    visibilityState: initialVisibility,
    addEventListener(type, listener) {
      listeners.set(type, listener)
    },
    removeEventListener(type, listener) {
      if (listeners.get(type) === listener) listeners.delete(type)
    },
  }
  const timers = {
    setInterval(callback, delay) {
      const id = nextIntervalId++
      intervals.set(id, { callback, delay })
      return id
    },
    clearInterval(id) {
      intervals.delete(id)
    },
  }

  return {
    documentRef,
    timers,
    intervals,
    setVisibility(state) {
      documentRef.visibilityState = state
      listeners.get('visibilitychange')?.()
    },
    tick() {
      for (const { callback } of intervals.values()) callback()
    },
    hasVisibilityListener() {
      return listeners.has('visibilitychange')
    },
  }
}

describe('startAutoReload', () => {
  it('reloads every 60 seconds while visible without an immediate duplicate load', () => {
    const h = harness()
    const reload = vi.fn()

    const stop = startAutoReload(reload, {
      documentRef: h.documentRef,
      timers: h.timers,
    })

    expect(reload).not.toHaveBeenCalled()
    expect(h.intervals.size).toBe(1)
    expect([...h.intervals.values()][0].delay).toBe(AUTO_RELOAD_INTERVAL_MS)

    h.tick()
    expect(reload).toHaveBeenCalledOnce()
    stop()
  })

  it('stops while hidden and reloads immediately when the tab becomes visible', () => {
    const h = harness()
    const reload = vi.fn()
    const stop = startAutoReload(reload, {
      documentRef: h.documentRef,
      timers: h.timers,
    })

    h.setVisibility('hidden')
    expect(h.intervals.size).toBe(0)
    h.tick()
    expect(reload).not.toHaveBeenCalled()

    h.setVisibility('visible')
    expect(reload).toHaveBeenCalledOnce()
    expect(h.intervals.size).toBe(1)
    h.tick()
    expect(reload).toHaveBeenCalledTimes(2)
    stop()
  })

  it('starts paused when hidden and cleanup removes all browser hooks', () => {
    const h = harness('hidden')
    const reload = vi.fn()
    const stop = startAutoReload(reload, {
      documentRef: h.documentRef,
      timers: h.timers,
    })

    expect(h.intervals.size).toBe(0)
    expect(h.hasVisibilityListener()).toBe(true)

    stop()
    expect(h.intervals.size).toBe(0)
    expect(h.hasVisibilityListener()).toBe(false)
    h.setVisibility('visible')
    expect(reload).not.toHaveBeenCalled()
  })
})
