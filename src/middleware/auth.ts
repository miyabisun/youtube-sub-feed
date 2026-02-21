import { createMiddleware } from 'hono/factory'
import { getCookie } from 'hono/cookie'
import { getSession } from '../lib/auth.js'

export const authMiddleware = createMiddleware(async (c, next) => {
  const sessionId = getCookie(c, 'session')
  if (!sessionId) return c.json({ error: 'Unauthorized' }, 401)

  const session = getSession(sessionId)
  if (!session) return c.json({ error: 'Unauthorized' }, 401)

  await next()
})
