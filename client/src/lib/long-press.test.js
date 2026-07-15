import { afterEach, describe, expect, test, vi } from 'vitest'
import { createLongPress, LONG_PRESS_MS } from './long-press.js'

afterEach(() => vi.useRealTimers())

describe('createLongPress', () => {
  test('opens after a 500ms touch press and consumes the following click', () => {
    vi.useFakeTimers()
    const open = vi.fn()
    const press = createLongPress(open)

    press.pointerDown({ pointerType: 'touch', clientX: 10, clientY: 20 }, 'channel')
    vi.advanceTimersByTime(LONG_PRESS_MS - 1)
    expect(open).not.toHaveBeenCalled()

    vi.advanceTimersByTime(1)
    expect(open).toHaveBeenCalledWith('channel')
    expect(press.consumeClick()).toBe(true)
    expect(press.consumeClick()).toBe(false)
  })

  test('ignores mouse presses because right-click uses contextmenu', () => {
    vi.useFakeTimers()
    const open = vi.fn()
    const press = createLongPress(open)

    press.pointerDown({ pointerType: 'mouse', clientX: 0, clientY: 0 }, 'channel')
    vi.advanceTimersByTime(LONG_PRESS_MS)

    expect(open).not.toHaveBeenCalled()
  })

  test('cancels when the pointer moves far enough to be a scroll gesture', () => {
    vi.useFakeTimers()
    const open = vi.fn()
    const press = createLongPress(open)

    press.pointerDown({ pointerType: 'touch', clientX: 10, clientY: 20 }, 'channel')
    press.pointerMove({ clientX: 10, clientY: 31 })
    vi.advanceTimersByTime(LONG_PRESS_MS)

    expect(open).not.toHaveBeenCalled()
    expect(press.consumeClick()).toBe(false)
  })

  test('drops click suppression when the browser cancels the pointer', () => {
    vi.useFakeTimers()
    const open = vi.fn()
    const press = createLongPress(open)

    press.pointerDown({ pointerType: 'touch', clientX: 10, clientY: 20 }, 'channel')
    vi.advanceTimersByTime(LONG_PRESS_MS)
    expect(open).toHaveBeenCalledOnce()

    press.abort()
    expect(press.consumeClick()).toBe(false)
  })
})
