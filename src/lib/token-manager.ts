import { sqlite } from '../db/index.js'
import { refreshAccessToken } from './google-oauth.js'
import type { Database } from 'bun:sqlite'

const REFRESH_MARGIN_MS = 5 * 60 * 1000 // 5 minutes before expiry

export async function getValidAccessTokenCore(
  db: Database,
  refreshFn: (refreshToken: string) => Promise<{ access_token: string; expires_in: number }>,
): Promise<string | null> {
  const authRow = db.query('SELECT id, access_token, refresh_token, token_expires_at FROM auth LIMIT 1').get() as {
    id: number
    access_token: string | null
    refresh_token: string | null
    token_expires_at: string | null
  } | null

  if (!authRow || !authRow.access_token || !authRow.refresh_token) return null

  const expiresAt = authRow.token_expires_at ? new Date(authRow.token_expires_at).getTime() : 0
  const now = Date.now()

  if (expiresAt - now > REFRESH_MARGIN_MS) {
    return authRow.access_token
  }

  // Token expired or about to expire - refresh
  try {
    const result = await refreshFn(authRow.refresh_token)
    const newExpiresAt = new Date(now + result.expires_in * 1000).toISOString()
    const updatedAt = new Date().toISOString()

    db.query('UPDATE auth SET access_token = ?, token_expires_at = ?, updated_at = ? WHERE id = ?').run(result.access_token, newExpiresAt, updatedAt, authRow.id)

    return result.access_token
  } catch (e) {
    console.error('[token-manager] Failed to refresh token:', e)
    return null
  }
}

export async function getValidAccessToken(): Promise<string | null> {
  return getValidAccessTokenCore(sqlite, refreshAccessToken)
}
