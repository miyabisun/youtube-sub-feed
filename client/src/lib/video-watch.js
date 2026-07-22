export function isWatchActivation(event) {
  if (event.type === 'click') return event.button === 0
  return event.type === 'auxclick' && event.button === 1
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
