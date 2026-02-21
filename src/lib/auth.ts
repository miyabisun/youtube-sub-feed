import { sqlite } from '../db/index.js'

const SESSION_TTL_DAYS = 30

export function createSession(authId: number): { sessionId: string; expiresAt: string } {
  const sessionId = crypto.randomUUID()
  const now = new Date()
  const expiresAt = new Date(now.getTime() + SESSION_TTL_DAYS * 24 * 60 * 60 * 1000).toISOString()
  const createdAt = now.toISOString()

  sqlite.query('INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES (?, ?, ?, ?)').run(sessionId, authId, expiresAt, createdAt)

  return { sessionId, expiresAt }
}

export function getSession(sessionId: string): { id: string; auth_id: number; expires_at: string } | null {
  const row = sqlite.query('SELECT id, auth_id, expires_at FROM sessions WHERE id = ?').get(sessionId) as { id: string; auth_id: number; expires_at: string } | null
  if (!row) return null
  if (new Date(row.expires_at) < new Date()) {
    deleteSession(sessionId)
    return null
  }
  return row
}

export function deleteSession(sessionId: string): void {
  sqlite.query('DELETE FROM sessions WHERE id = ?').run(sessionId)
}
