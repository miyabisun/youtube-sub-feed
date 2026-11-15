// Requests the browser makes to YouTube as the viewer, with their own
// short-lived Google token. Comments and ratings both go through here.
export const API = 'https://www.googleapis.com/youtube/v3'

/** Needed to comment and to rate; reading the subscription list needs less. */
export const WRITE_SCOPE = 'https://www.googleapis.com/auth/youtube.force-ssl'

export class WriteError extends Error {
  constructor(status, reason) {
    super(`${status} ${reason}`)
    this.status = status
    this.reason = reason
  }
}

const REASONS = {
  quotaExceeded: '今日の API の割り当てを使い切りました。明日もう一度試してください',
  insufficientPermissions: 'Google の認可が足りません。もう一度試して認可し直してください',
  videoNotFound: '動画が見つかりません',
}

/** What to tell the viewer. `reasons` adds or replaces wording for the caller's own API reasons. */
export function writeErrorMessage(error, reasons = {}) {
  if (error instanceof WriteError) {
    const known = reasons[error.reason] ?? REASONS[error.reason]
    if (known) return known
    if (error.status === 401) return 'Google の認可が切れました。もう一度試してください'
    return `送信できませんでした (${error.status} ${error.reason})`
  }
  const message = error?.message ?? ''
  if (message.includes('cancelled') || message.includes('popup_closed')) {
    return 'Google の認可がキャンセルされました'
  }
  if (message.startsWith('GIS ')) return message
  return '送信できませんでした。通信を確かめてください'
}

/**
 * Send one request with the viewer's token. Resolves to the response body
 * (null when the API answers without one), rejects with WriteError.
 */
export async function sendAsViewer({ url, init }, { tokens, fetchFn = fetch }) {
  const token = await tokens.get()
  const headers = { Authorization: `Bearer ${token}` }
  if (init.body) headers['Content-Type'] = 'application/json'
  const response = await fetchFn(url, { ...init, headers })
  const body = await response.json().catch(() => null)
  if (response.ok) return body
  if (response.status === 401) tokens.invalidate()
  throw new WriteError(response.status, body?.error?.errors?.[0]?.reason ?? 'unknown')
}
