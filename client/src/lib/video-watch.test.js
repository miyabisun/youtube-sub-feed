import { describe, expect, it, vi } from 'vitest'
import { createVideoHider, isWatchActivation } from './video-watch.js'

describe('isWatchActivation', () => {
  it('accepts primary clicks including modifier clicks', () => {
    expect(isWatchActivation({ type: 'click', button: 0 })).toBe(true)
    expect(isWatchActivation({ type: 'click', button: 0, ctrlKey: true })).toBe(true)
    expect(isWatchActivation({ type: 'click', button: 0, metaKey: true })).toBe(true)
  })

  it('accepts middle clicks and rejects right clicks', () => {
    expect(isWatchActivation({ type: 'auxclick', button: 1 })).toBe(true)
    expect(isWatchActivation({ type: 'auxclick', button: 2 })).toBe(false)
    expect(isWatchActivation({ type: 'contextmenu', button: 2 })).toBe(false)
  })
})

describe('createVideoHider', () => {
  function harness(hideRequest = vi.fn().mockResolvedValue({ ok: true })) {
    const onHidden = vi.fn()
    const onSuccess = vi.fn()
    const onError = vi.fn()
    return {
      hideRequest,
      onHidden,
      onSuccess,
      onError,
      hideVideo: createVideoHider({ hideRequest, onHidden, onSuccess, onError }),
    }
  }

  it('uses a keepalive PATCH and suppresses success feedback for automatic marking', async () => {
    const h = harness()

    await expect(h.hideVideo('v1', { silent: true })).resolves.toBe(true)

    expect(h.hideRequest).toHaveBeenCalledWith('v1', { method: 'PATCH', keepalive: true })
    expect(h.onHidden).toHaveBeenCalledWith('v1')
    expect(h.onSuccess).not.toHaveBeenCalled()
    expect(h.onError).not.toHaveBeenCalled()
  })

  it('reports manual success and ignores a duplicate request while one is pending', async () => {
    let resolveRequest
    const hideRequest = vi.fn(
      () =>
        new Promise((resolve) => {
          resolveRequest = resolve
        }),
    )
    const h = harness(hideRequest)

    const first = h.hideVideo('v1')
    await expect(h.hideVideo('v1')).resolves.toBe(false)
    expect(h.hideRequest).toHaveBeenCalledOnce()

    resolveRequest({ ok: true })
    await expect(first).resolves.toBe(true)
    expect(h.onSuccess).toHaveBeenCalledOnce()
  })

  it('keeps the video visible and reports an error when marking fails', async () => {
    const error = new Error('failed')
    const h = harness(vi.fn().mockRejectedValue(error))

    await expect(h.hideVideo('v1')).resolves.toBe(false)

    expect(h.onHidden).not.toHaveBeenCalled()
    expect(h.onSuccess).not.toHaveBeenCalled()
    expect(h.onError).toHaveBeenCalledWith(error)
  })
})
