// The pages against stand-ins: this app's /api, YouTube and Google are all
// answered here, in every tab the page opens, so nothing leaves the machine.
import { expect } from '@playwright/test'

export const BASE = 'http://127.0.0.1:4173'
export const IDS = [
  'aaaaaaaaaa1',
  'bbbbbbbbbb2',
  'cccccccccc3',
  'dddddddddd4',
  'eeeeeeeeee5',
  'ffffffffff6',
  'gggggggggg7',
  'hhhhhhhhhh8',
]
const feed = () =>
  IDS.map((id, i) => ({
    kind: 'video',
    id,
    channel_id: 'UC' + 'x'.repeat(22),
    channel_title: 'とても長いチャンネルの名前 '.repeat(i === 0 ? 4 : 1) + i,
    title: (i === 0 ? '長い長いタイトルの動画 '.repeat(8) : '動画 ') + i,
    published_at: '2026-11-14T00:00:00Z',
    duration: 'PT10M',
    is_short: i === 2 ? 1 : 0,
    is_livestream: i === 3 || i === 4 ? 1 : 0,
    livestream_ended_at: i === 4 ? '2026-11-14T01:00:00Z' : null,
  }))
const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==',
  'base64',
)
export const check = (name, ok, detail = '') =>
  expect.soft(ok, detail ? `${name} — ${detail}` : name).toBe(true)

export async function setup(
  browser,
  {
    width = 400,
    height = 800,
    scheme = 'dark',
    gisClientId = 'test-client',
    groups = [],
    channels = [],
    failSettings = false,
  } = {},
) {
  const context = await browser.newContext({
    viewport: { width, height },
    colorScheme: scheme,
    hasTouch: width < 768,
    isMobile: width < 768,
  })
  const page = await context.newPage()
  const log = { hide: [], feed: 0, youtube: [], settings: [] }
  page.on('pageerror', (error) => expect.soft(error.message, 'uncaught page error').toBe(''))
  await context.route('**/*', async (route) => {
    const req = route.request()
    const url = new URL(req.url())
    if (url.origin === BASE) {
      if (url.pathname === '/api/auth/me')
        return route.fulfill({ json: { email: 'me@example.test' } })
      if (url.pathname === '/api/groups') return route.fulfill({ json: groups })
      if (url.pathname === '/api/channels') return route.fulfill({ json: channels })
      // A channel's settings: kept here as the server keeps them, so a reload shows them.
      const settings = url.pathname.match(/^\/api\/channels\/([\w-]+)$/)
      if (settings && req.method() === 'PATCH') {
        const body = req.postDataJSON()
        log.settings.push({ id: settings[1], body })
        if (failSettings) return route.fulfill({ status: 500, json: { error: 'refused' } })
        Object.assign(channels.find((c) => c.id === settings[1]) ?? {}, body)
        return route.fulfill({ json: { ok: true } })
      }
      if (/^\/api\/channels\/[\w-]+\/videos$/.test(url.pathname))
        return route.fulfill({
          json: (url.searchParams.get('offset') ?? '0') === '0' ? feed().slice(0, 3) : [],
        })
      if (/^\/api\/groups\/\d+\/channels$/.test(url.pathname)) return route.fulfill({ json: [] })
      if (url.pathname === '/api/feed') {
        log.feed++
        return route.fulfill({
          json:
            url.searchParams.get('offset') === '0'
              ? feed().filter((v) => !log.hide.includes(v.id))
              : [],
        })
      }
      if (url.pathname === '/api/history')
        return route.fulfill({
          json:
            url.searchParams.get('offset') === '0' || !url.searchParams.get('offset')
              ? feed()
                  .slice(0, 2)
                  .map((v) => ({ ...v, watched_at: '2026-11-15T00:00:00Z' }))
              : [],
        })
      const m = url.pathname.match(/^\/api\/videos\/([\w-]+)\/hide$/)
      if (m) {
        log.hide.push(m[1])
        return route.fulfill({ json: { ok: true } })
      }
      if (url.pathname.startsWith('/api/'))
        return route.fulfill({ status: 404, json: { error: 'unstubbed ' + url.pathname } })
      if (req.resourceType() === 'document') {
        const res = await route.fetch()
        const html = (await res.text()).replace(
          "window.__GIS_CLIENT_ID__ = ''",
          `window.__GIS_CLIENT_ID__ = '${gisClientId}'`,
        )
        return route.fulfill({ response: res, body: html })
      }
      return route.continue()
    }
    // Where a video link leads. A tab opened in the background never reports
    // its address, so the request it makes is the record of what was opened.
    if (url.hostname === 'www.youtube.com' && req.resourceType() === 'document') {
      log.youtube.push(url.href)
      return route.fulfill({ contentType: 'text/html', body: 'YOUTUBE' })
    }
    if (url.hostname === 'accounts.google.com')
      return route.fulfill({ contentType: 'text/javascript', body: '' })
    if (url.hostname.endsWith('ytimg.com') || url.hostname.endsWith('ggpht.com'))
      return route.fulfill({ contentType: 'image/png', body: PNG })
    return route.abort()
  })
  return { context, page, log }
}

export const noHScroll = (page) =>
  page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)
