import { Hono } from 'hono'
import { logger } from 'hono/logger'
import { serveStatic } from 'hono/bun'

import init from './lib/init.js'
import { getIndexHtml } from './lib/spa.js'
import authRoutes from './routes/auth.js'
import feedRoutes from './routes/feed.js'
import channelRoutes from './routes/channels.js'
import groupRoutes from './routes/groups.js'
import { authMiddleware } from './middleware/auth.js'
import { startNormalPolling, startFastPolling, startDailySync } from './lib/polling.js'
import { initDiscordClient } from './lib/discord.js'

const port = Number(process.env.PORT) || 3000

const app = new Hono()
app.use('*', logger())

// Health check (no auth)
app.get('/api/health', (c) => c.json({ ok: true }))

// Auth routes (no auth middleware)
app.route('/', authRoutes)

// All other /api/* routes require auth
app.use('/api/*', authMiddleware)
app.route('/', feedRoutes)
app.route('/', channelRoutes)
app.route('/', groupRoutes)

// Static files & SPA fallback
app.use('/assets/*', serveStatic({ root: './client/build' }))
app.get('*', (c) => {
  const html = getIndexHtml()
  if (html) return c.html(html)
  return c.json({ error: 'Frontend not built. Run: bun run build:client' }, 404)
})

init()
initDiscordClient()
startNormalPolling()
startFastPolling()
startDailySync()

console.log(`Server running on http://localhost:${port}`)

export default { port, fetch: app.fetch }
