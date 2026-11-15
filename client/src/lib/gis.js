// Google Identity Services token client: short-lived, online-only access
// tokens. No refresh token is ever requested, and no token leaves the browser.

/** Ask Google for an access token. Opens the consent popup when needed. */
export function requestGisToken(clientId, scope) {
  return new Promise((resolve, reject) => {
    if (!clientId) {
      reject(new Error('GIS_CLIENT_ID が設定されていません'))
      return
    }
    if (!window.google?.accounts?.oauth2) {
      reject(new Error('GIS SDK がロードされていません。ページをリロードしてください。'))
      return
    }
    const client = window.google.accounts.oauth2.initTokenClient({
      client_id: clientId,
      scope,
      // Ask only the first time: once access is granted, the popup closes by
      // itself instead of making the viewer pick their account on every visit.
      prompt: '',
      callback: (response) => {
        if (response.error) {
          reject(new Error(response.error_description || response.error))
        } else {
          resolve({
            accessToken: response.access_token,
            expiresIn: Number(response.expires_in) || 0,
          })
        }
      },
      error_callback: (err) => {
        reject(new Error(err?.message || 'token request cancelled'))
      },
    })
    client.requestAccessToken()
  })
}

/** Renew this long before the stated expiry, so a request never starts with a dying token. */
const EXPIRY_MARGIN_MS = 60_000

/**
 * Keep one token for as long as Google says it lives, so the consent popup
 * appears once per visit rather than once per comment.
 */
export function createTokenCache(request, now = Date.now) {
  let token = null
  let expiresAt = 0

  return {
    async get() {
      if (token && now() < expiresAt) return token
      const { accessToken, expiresIn } = await request()
      token = accessToken
      expiresAt = now() + expiresIn * 1000 - EXPIRY_MARGIN_MS
      return token
    },
    /** The token if one is live, without ever opening the consent popup. */
    peek() {
      return token && now() < expiresAt ? token : null
    },
    invalidate() {
      token = null
    },
  }
}
