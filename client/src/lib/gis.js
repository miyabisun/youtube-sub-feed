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
          resolve(response.access_token)
        }
      },
      error_callback: (err) => {
        reject(new Error(err?.message || 'token request cancelled'))
      },
    })
    client.requestAccessToken()
  })
}
