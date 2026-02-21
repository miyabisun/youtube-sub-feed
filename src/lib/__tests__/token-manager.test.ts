import { describe, test, expect } from 'bun:test'
import { Database } from 'bun:sqlite'
import { getValidAccessTokenCore } from '../token-manager.js'

function setupDb() {
  const sqlite = new Database(':memory:')
  sqlite.exec(`CREATE TABLE auth (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    google_id TEXT NOT NULL,
    email TEXT NOT NULL,
    access_token TEXT,
    refresh_token TEXT,
    token_expires_at TEXT,
    updated_at TEXT
  )`)
  return sqlite
}

describe('getValidAccessTokenCore', () => {
  test('returns null when no auth record exists', async () => {
    const db = setupDb()
    const token = await getValidAccessTokenCore(db, async () => ({ access_token: 'new', expires_in: 3600 }))
    expect(token).toBeNull()
  })

  test('returns null when access_token is null', async () => {
    const db = setupDb()
    db.exec("INSERT INTO auth (google_id, email, refresh_token) VALUES ('g1', 'a@b.com', 'rt')")
    const token = await getValidAccessTokenCore(db, async () => ({ access_token: 'new', expires_in: 3600 }))
    expect(token).toBeNull()
  })

  test('returns null when refresh_token is null', async () => {
    const db = setupDb()
    db.exec("INSERT INTO auth (google_id, email, access_token) VALUES ('g1', 'a@b.com', 'at')")
    const token = await getValidAccessTokenCore(db, async () => ({ access_token: 'new', expires_in: 3600 }))
    expect(token).toBeNull()
  })

  test('returns existing token when not expired (more than 5min remaining)', async () => {
    const db = setupDb()
    const future = new Date(Date.now() + 30 * 60 * 1000).toISOString() // 30 min from now
    db.exec(`INSERT INTO auth (google_id, email, access_token, refresh_token, token_expires_at) VALUES ('g1', 'a@b.com', 'valid_token', 'rt', '${future}')`)

    let refreshCalled = false
    const token = await getValidAccessTokenCore(db, async () => { refreshCalled = true; return { access_token: 'new', expires_in: 3600 } })
    expect(token).toBe('valid_token')
    expect(refreshCalled).toBe(false)
  })

  test('refreshes token when less than 5 minutes remaining', async () => {
    const db = setupDb()
    const nearExpiry = new Date(Date.now() + 2 * 60 * 1000).toISOString() // 2 min from now
    db.exec(`INSERT INTO auth (google_id, email, access_token, refresh_token, token_expires_at) VALUES ('g1', 'a@b.com', 'old_token', 'rt', '${nearExpiry}')`)

    const token = await getValidAccessTokenCore(db, async () => ({ access_token: 'refreshed_token', expires_in: 3600 }))
    expect(token).toBe('refreshed_token')

    const row = db.query('SELECT access_token FROM auth LIMIT 1').get() as any
    expect(row.access_token).toBe('refreshed_token')
  })

  test('refreshes token when already expired', async () => {
    const db = setupDb()
    const past = new Date(Date.now() - 60 * 1000).toISOString() // 1 min ago
    db.exec(`INSERT INTO auth (google_id, email, access_token, refresh_token, token_expires_at) VALUES ('g1', 'a@b.com', 'expired', 'rt', '${past}')`)

    const token = await getValidAccessTokenCore(db, async () => ({ access_token: 'new_token', expires_in: 3600 }))
    expect(token).toBe('new_token')
  })

  test('refreshes when token_expires_at is null', async () => {
    const db = setupDb()
    db.exec("INSERT INTO auth (google_id, email, access_token, refresh_token) VALUES ('g1', 'a@b.com', 'old', 'rt')")

    const token = await getValidAccessTokenCore(db, async () => ({ access_token: 'new', expires_in: 3600 }))
    expect(token).toBe('new')
  })

  test('returns null when refresh fails', async () => {
    const db = setupDb()
    const past = new Date(Date.now() - 60 * 1000).toISOString()
    db.exec(`INSERT INTO auth (google_id, email, access_token, refresh_token, token_expires_at) VALUES ('g1', 'a@b.com', 'expired', 'rt', '${past}')`)

    const token = await getValidAccessTokenCore(db, async () => { throw new Error('refresh failed') })
    expect(token).toBeNull()
  })
})
