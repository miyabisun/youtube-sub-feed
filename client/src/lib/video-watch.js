export function isWatchActivation(event) {
  if (event.type === 'click') return event.button === 0
  return event.type === 'auxclick' && event.button === 1
}

/** A plain tap plays in the app; modifier and middle clicks keep opening YouTube in a new tab. */
export function opensInApp(event) {
  return (
    event.type === 'click' &&
    event.button === 0 &&
    !(event.ctrlKey || event.metaKey || event.shiftKey || event.altKey)
  )
}

export function createVideoHider({ hideRequest, onHidden, onSuccess, onError }) {
  const pending = new Set()

  return async function hideVideo(id, { silent = false } = {}) {
    if (pending.has(id)) return false
    pending.add(id)

    try {
      await hideRequest(id, { method: 'PATCH', keepalive: true })
      onHidden(id)
      if (!silent) onSuccess()
      return true
    } catch (error) {
      onError(error)
      return false
    } finally {
      pending.delete(id)
    }
  }
}
