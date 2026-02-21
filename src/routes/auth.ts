import { Hono } from 'hono'
import { getCookie, setCookie, deleteCookie } from 'hono/cookie'
import { sqlite } from '../db/index.js'
import { getAuthUrl, exchangeCode, getGoogleUserInfo } from '../lib/google-oauth.js'
import { createSession, getSession, deleteSession } from '../lib/auth.js'

const auth = new Hono()

auth.get('/api/auth/login', (c) => {
  const state = crypto.randomUUID()
  const isProduction = process.env.NODE_ENV === 'production'
  setCookie(c, 'oauth_state', state, {
    httpOnly: true,
    sameSite: 'Lax',
    path: '/',
    secure: isProduction,
    maxAge: 600, // 10 minutes
  })
  const url = getAuthUrl(state)
  return c.redirect(url)
})

auth.get('/api/auth/callback', async (c) => {
  const code = c.req.query('code')
  if (!code) return c.json({ error: 'Missing code' }, 400)

  const state = c.req.query('state')
  const savedState = getCookie(c, 'oauth_state')
  deleteCookie(c, 'oauth_state', { path: '/' })
  if (!state || !savedState || state !== savedState) {
    return c.json({ error: 'Invalid state' }, 400)
  }

  try {
    const tokens = await exchangeCode(code)
    const userInfo = await getGoogleUserInfo(tokens.access_token)

    const now = new Date()
    const tokenExpiresAt = new Date(now.getTime() + tokens.expires_in * 1000).toISOString()
    const updatedAt = now.toISOString()

    // Upsert auth record
    const existing = sqlite.query('SELECT id FROM auth WHERE google_id = ?').get(userInfo.id) as { id: number } | null

    let authId: number
    if (existing) {
      authId = existing.id
      sqlite.query('UPDATE auth SET email = ?, access_token = ?, refresh_token = ?, token_expires_at = ?, updated_at = ? WHERE id = ?').run(userInfo.email, tokens.access_token, tokens.refresh_token, tokenExpiresAt, updatedAt, authId)
    } else {
      sqlite.query('INSERT INTO auth (google_id, email, access_token, refresh_token, token_expires_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)').run(userInfo.id, userInfo.email, tokens.access_token, tokens.refresh_token, tokenExpiresAt, updatedAt)
      const row = sqlite.query('SELECT id FROM auth WHERE google_id = ?').get(userInfo.id) as { id: number }
      authId = row.id
    }

    const session = createSession(authId)

    const isProduction = process.env.NODE_ENV === 'production'
    setCookie(c, 'session', session.sessionId, {
      httpOnly: true,
      sameSite: 'Lax',
      path: '/',
      secure: isProduction,
      expires: new Date(session.expiresAt),
    })

    return c.redirect('/')
  } catch (e) {
    console.error('[auth] Callback error:', e)
    return c.json({ error: 'Authentication failed' }, 500)
  }
})

auth.post('/api/auth/logout', (c) => {
  const sessionId = getCookie(c, 'session')
  if (sessionId) {
    deleteSession(sessionId)
    deleteCookie(c, 'session', { path: '/' })
  }
  return c.json({ ok: true })
})

auth.get('/api/auth/me', (c) => {
  const sessionId = getCookie(c, 'session')
  if (!sessionId) return c.json({ error: 'Unauthorized' }, 401)

  const session = getSession(sessionId)
  if (!session) return c.json({ error: 'Unauthorized' }, 401)

  const user = sqlite.query('SELECT email FROM auth WHERE id = ?').get(session.auth_id) as { email: string } | null
  if (!user) return c.json({ error: 'Unauthorized' }, 401)

  return c.json({ email: user.email })
})

export default auth
