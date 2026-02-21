import { describe, test, expect } from 'bun:test'
import { Database } from 'bun:sqlite'

function setupTestDb() {
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
  return sqlite
}

function createSession(sqlite: Database, authId: number): { sessionId: string; expiresAt: string } {
  const sessionId = crypto.randomUUID()
  const now = new Date()
  const expiresAt = new Date(now.getTime() + 30 * 24 * 60 * 60 * 1000).toISOString()
  const createdAt = now.toISOString()
  sqlite.query('INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES (?, ?, ?, ?)').run(sessionId, authId, expiresAt, createdAt)
  return { sessionId, expiresAt }
}

function getSession(sqlite: Database, sessionId: string) {
  const row = sqlite.query('SELECT id, auth_id, expires_at FROM sessions WHERE id = ?').get(sessionId) as { id: string; auth_id: number; expires_at: string } | null
  if (!row) return null
  if (new Date(row.expires_at) < new Date()) {
    sqlite.query('DELETE FROM sessions WHERE id = ?').run(sessionId)
    return null
  }
  return row
}

function deleteSession(sqlite: Database, sessionId: string) {
  sqlite.query('DELETE FROM sessions WHERE id = ?').run(sessionId)
}

describe('session management', () => {
  test('creates a session with UUID and 30-day expiry', () => {
    const sqlite = setupTestDb()
    sqlite.exec("INSERT INTO auth (google_id, email) VALUES ('g1', 'test@example.com')")

    const session = createSession(sqlite, 1)
    expect(session.sessionId).toMatch(/^[0-9a-f-]{36}$/)

    const expiresAt = new Date(session.expiresAt)
    const now = new Date()
    const diffDays = (expiresAt.getTime() - now.getTime()) / (1000 * 60 * 60 * 24)
    expect(diffDays).toBeGreaterThan(29)
    expect(diffDays).toBeLessThanOrEqual(30)
  })

  test('retrieves a valid session', () => {
    const sqlite = setupTestDb()
    sqlite.exec("INSERT INTO auth (google_id, email) VALUES ('g1', 'test@example.com')")

    const session = createSession(sqlite, 1)
    const retrieved = getSession(sqlite, session.sessionId)
    expect(retrieved).not.toBeNull()
    expect(retrieved!.auth_id).toBe(1)
  })

  test('returns null for non-existent session', () => {
    const sqlite = setupTestDb()
    const retrieved = getSession(sqlite, 'nonexistent-id')
    expect(retrieved).toBeNull()
  })

  test('returns null and deletes expired session', () => {
    const sqlite = setupTestDb()
    sqlite.exec("INSERT INTO auth (google_id, email) VALUES ('g1', 'test@example.com')")

    const pastDate = new Date(Date.now() - 1000).toISOString()
    sqlite.query('INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES (?, ?, ?, ?)').run('expired-sess', 1, pastDate, pastDate)

    const retrieved = getSession(sqlite, 'expired-sess')
    expect(retrieved).toBeNull()

    // Verify it was deleted
    const row = sqlite.query("SELECT id FROM sessions WHERE id = 'expired-sess'").get()
    expect(row).toBeNull()
  })

  test('deletes a session', () => {
    const sqlite = setupTestDb()
    sqlite.exec("INSERT INTO auth (google_id, email) VALUES ('g1', 'test@example.com')")

    const session = createSession(sqlite, 1)
    deleteSession(sqlite, session.sessionId)

    const retrieved = getSession(sqlite, session.sessionId)
    expect(retrieved).toBeNull()
  })

  test('sessions cascade on auth delete', () => {
    const sqlite = setupTestDb()
    sqlite.exec("INSERT INTO auth (google_id, email) VALUES ('g1', 'test@example.com')")

    createSession(sqlite, 1)
    createSession(sqlite, 1)

    sqlite.exec('DELETE FROM auth WHERE id = 1')
    const sessions = sqlite.query('SELECT id FROM sessions WHERE auth_id = 1').all()
    expect(sessions).toHaveLength(0)
  })
})
