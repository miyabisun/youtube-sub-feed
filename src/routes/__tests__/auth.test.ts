import { describe, test, expect } from 'bun:test'
import { Hono } from 'hono'
import { getCookie, setCookie, deleteCookie } from 'hono/cookie'
import { Database } from 'bun:sqlite'

function setupApp() {
  const sqlite = new Database(':memory:')
  sqlite.exec('PRAGMA foreign_keys = ON')

  sqlite.exec(`CREATE TABLE auth (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    google_id TEXT NOT NULL,
    email TEXT NOT NULL,
    access_token TEXT,
    refresh_token TEXT,
    token_expires_at TEXT,
    updated_at TEXT
  )`)
  sqlite.exec(`CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    auth_id INTEGER NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (auth_id) REFERENCES auth(id) ON DELETE CASCADE
  )`)

  const app = new Hono()

  // GET /api/auth/me
  app.get('/api/auth/me', (c) => {
    const sessionId = getCookie(c, 'session')
    if (!sessionId) return c.json({ error: 'Unauthorized' }, 401)

    const session = sqlite.query('SELECT id, auth_id, expires_at FROM sessions WHERE id = ?').get(sessionId) as { id: string; auth_id: number; expires_at: string } | null
    if (!session || new Date(session.expires_at) < new Date()) {
      return c.json({ error: 'Unauthorized' }, 401)
    }

    const user = sqlite.query('SELECT email FROM auth WHERE id = ?').get(session.auth_id) as { email: string } | null
    if (!user) return c.json({ error: 'Unauthorized' }, 401)

    return c.json({ email: user.email })
  })

  // POST /api/auth/logout
  app.post('/api/auth/logout', (c) => {
    const sessionId = getCookie(c, 'session')
    if (sessionId) {
      sqlite.query('DELETE FROM sessions WHERE id = ?').run(sessionId)
    }
    return c.json({ ok: true })
  })

  // Protected test route
  app.get('/api/protected', (c) => {
    const sessionId = getCookie(c, 'session')
    if (!sessionId) return c.json({ error: 'Unauthorized' }, 401)

    const session = sqlite.query('SELECT id, auth_id, expires_at FROM sessions WHERE id = ?').get(sessionId) as { id: string; auth_id: number; expires_at: string } | null
    if (!session || new Date(session.expires_at) < new Date()) {
      return c.json({ error: 'Unauthorized' }, 401)
    }

    return c.json({ ok: true })
  })

  return { app, sqlite }
}

function createTestSession(sqlite: Database): string {
  sqlite.exec("INSERT OR IGNORE INTO auth (id, google_id, email) VALUES (1, 'g1', 'test@example.com')")
  const sessionId = crypto.randomUUID()
  const expiresAt = new Date(Date.now() + 30 * 24 * 60 * 60 * 1000).toISOString()
  sqlite.query('INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES (?, ?, ?, ?)').run(sessionId, 1, expiresAt, new Date().toISOString())
  return sessionId
}

describe('GET /api/auth/me', () => {
  test('returns 401 without session cookie', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/auth/me')
    expect(res.status).toBe(401)
  })

  test('returns 401 with invalid session', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/auth/me', {
      headers: { Cookie: 'session=invalid-session-id' },
    })
    expect(res.status).toBe(401)
  })

  test('returns 200 with valid session', async () => {
    const { app, sqlite } = setupApp()
    const sessionId = createTestSession(sqlite)
    const res = await app.request('/api/auth/me', {
      headers: { Cookie: `session=${sessionId}` },
    })
    expect(res.status).toBe(200)
    const body = await res.json()
    expect(body.email).toBe('test@example.com')
  })

  test('returns 401 with expired session', async () => {
    const { app, sqlite } = setupApp()
    sqlite.exec("INSERT INTO auth (google_id, email) VALUES ('g1', 'test@example.com')")
    const pastDate = new Date(Date.now() - 1000).toISOString()
    sqlite.query('INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES (?, ?, ?, ?)').run('expired', 1, pastDate, pastDate)

    const res = await app.request('/api/auth/me', {
      headers: { Cookie: 'session=expired' },
    })
    expect(res.status).toBe(401)
  })
})

describe('POST /api/auth/logout', () => {
  test('deletes session and returns ok', async () => {
    const { app, sqlite } = setupApp()
    const sessionId = createTestSession(sqlite)

    const res = await app.request('/api/auth/logout', {
      method: 'POST',
      headers: { Cookie: `session=${sessionId}` },
    })
    expect(res.status).toBe(200)
    const body = await res.json()
    expect(body.ok).toBe(true)

    // Session should be gone
    const meRes = await app.request('/api/auth/me', {
      headers: { Cookie: `session=${sessionId}` },
    })
    expect(meRes.status).toBe(401)
  })

  test('returns ok even without session', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/auth/logout', { method: 'POST' })
    expect(res.status).toBe(200)
  })
})

function setupCallbackApp() {
  const app = new Hono()

  app.get('/api/auth/login', (c) => {
    const state = crypto.randomUUID()
    setCookie(c, 'oauth_state', state, {
      httpOnly: true,
      sameSite: 'Lax',
      path: '/',
      maxAge: 600,
    })
    return c.json({ url: `https://accounts.google.com/o/oauth2/v2/auth?state=${state}` })
  })

  app.get('/api/auth/callback', (c) => {
    const code = c.req.query('code')
    if (!code) return c.json({ error: 'Missing code' }, 400)

    const state = c.req.query('state')
    const savedState = getCookie(c, 'oauth_state')
    deleteCookie(c, 'oauth_state', { path: '/' })
    if (!state || !savedState || state !== savedState) {
      return c.json({ error: 'Invalid state' }, 400)
    }

    return c.json({ ok: true })
  })

  return app
}

describe('GET /api/auth/callback state validation', () => {
  test('returns 400 without state parameter', async () => {
    const app = setupCallbackApp()
    const res = await app.request('/api/auth/callback?code=authcode', {
      headers: { Cookie: 'oauth_state=some-state' },
    })
    expect(res.status).toBe(400)
    const body = await res.json()
    expect(body.error).toBe('Invalid state')
  })

  test('returns 400 when state does not match cookie', async () => {
    const app = setupCallbackApp()
    const res = await app.request('/api/auth/callback?code=authcode&state=wrong-state', {
      headers: { Cookie: 'oauth_state=correct-state' },
    })
    expect(res.status).toBe(400)
    const body = await res.json()
    expect(body.error).toBe('Invalid state')
  })

  test('returns 400 without oauth_state cookie', async () => {
    const app = setupCallbackApp()
    const res = await app.request('/api/auth/callback?code=authcode&state=some-state')
    expect(res.status).toBe(400)
    const body = await res.json()
    expect(body.error).toBe('Invalid state')
  })

  test('succeeds when state matches cookie', async () => {
    const app = setupCallbackApp()
    const res = await app.request('/api/auth/callback?code=authcode&state=valid-state', {
      headers: { Cookie: 'oauth_state=valid-state' },
    })
    expect(res.status).toBe(200)
  })

  test('login sets oauth_state cookie', async () => {
    const app = setupCallbackApp()
    const res = await app.request('/api/auth/login')
    expect(res.status).toBe(200)
    const setCookieHeader = res.headers.get('set-cookie')
    expect(setCookieHeader).toContain('oauth_state=')
  })
})

describe('protected route', () => {
  test('returns 401 without auth', async () => {
    const { app } = setupApp()
    const res = await app.request('/api/protected')
    expect(res.status).toBe(401)
  })

  test('returns 200 with valid session', async () => {
    const { app, sqlite } = setupApp()
    const sessionId = createTestSession(sqlite)
    const res = await app.request('/api/protected', {
      headers: { Cookie: `session=${sessionId}` },
    })
    expect(res.status).toBe(200)
  })
})
