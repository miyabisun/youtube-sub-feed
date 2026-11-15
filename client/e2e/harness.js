// The watch view against stand-ins: this app's /api, the YouTube player, Google
// Identity Services and the YouTube Data API are all answered here, so nothing
// leaves the machine and no comment is ever posted.
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
    is_livestream: 0,
    livestream_ended_at: null,
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
    gis = 'ok',
    gisClientId = 'test-client',
    comments = 'ok',
    post = 'ok',
    groups = [],
    channels = [],
  } = {},
) {
  const context = await browser.newContext({
    viewport: { width, height },
    colorScheme: scheme,
    hasTouch: width < 768,
    isMobile: width < 768,
  })
  const page = await context.newPage()
  const log = {
    hide: [],
    feed: 0,
    posts: [],
    puts: [],
    deletes: [],
    scopes: [],
    commentGets: [],
    state: { gis, comments, post },
  }
  page.on('pageerror', (error) => expect.soft(error.message, 'uncaught page error').toBe(''))
  await page.route('**/*', async (route) => {
    const req = route.request()
    const url = new URL(req.url())
    if (url.origin === BASE) {
      if (url.pathname === '/api/auth/me')
        return route.fulfill({ json: { email: 'me@example.test' } })
      if (url.pathname === '/api/groups') return route.fulfill({ json: groups })
      if (url.pathname === '/api/channels') return route.fulfill({ json: channels })
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
      let m = url.pathname.match(/^\/api\/videos\/([\w-]+)\/hide$/)
      if (m) {
        log.hide.push(m[1])
        return route.fulfill({ json: { ok: true } })
      }
      m = url.pathname.match(/^\/api\/videos\/([\w-]+)\/comments$/)
      if (m) {
        log.commentGets.push({ id: m[1], token: url.searchParams.get('page_token') })
        const s = log.state.comments
        if (s === 'error') return route.fulfill({ status: 502, json: { error: 'x' } })
        if (s !== 'ok')
          return route.fulfill({ json: { status: s, items: [], next_page_token: null } })
        const page2 = url.searchParams.get('page_token') === 'P2'
        return route.fulfill({
          json: {
            status: 'ok',
            next_page_token: page2 ? null : 'P2',
            items: [0, 1, 2].map((i) => ({
              id: `${m[1]}-c${page2 ? i + 3 : i}`,
              author: '@viewer' + i,
              author_avatar: 'https://yt3.ggpht.com/a',
              like_count: i * 7,
              published_at: '2026-11-10T00:00:00Z',
              text:
                i === 1 && !page2
                  ? Array.from({ length: 12 }, (_, n) => `長いコメントの ${n + 1} 行目`).join('\n')
                  : `コメント ${page2 ? i + 3 : i} <b>not html</b>`,
            })),
          },
        })
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
    if (url.hostname === 'accounts.google.com')
      return route.fulfill({
        contentType: 'text/javascript',
        body: `
      window.__gis = { scopes: [], mode: '${gis}' }
      window.google = { accounts: { oauth2: { initTokenClient: (cfg) => ({ requestAccessToken() {
        window.__gis.scopes.push(cfg.scope)
        if (window.__gis.mode === 'cancel') cfg.error_callback({ message: 'popup_closed' })
        else cfg.callback({ access_token: 'TOKEN', expires_in: 3600 })
      } }) } } }`,
      })
    if (url.hostname === 'www.youtube.com' && url.pathname === '/iframe_api')
      return route.fulfill({
        contentType: 'text/javascript',
        body: `
      window.__players = []
      window.YT = { Player: class { constructor(frame, opts) { this.frame = frame; this.opts = opts; window.__players.push(this)
        setTimeout(() => opts.events.onReady({ target: { getVideoData: () => ({ title: 'プレイヤーが知らせた題' }) } }), 0) } destroy() { this.destroyed = true } } }
      window.onYouTubeIframeAPIReady && window.onYouTubeIframeAPIReady()`,
      })
    if (url.hostname === 'www.youtube.com' && url.pathname.startsWith('/embed/'))
      return route.fulfill({
        contentType: 'text/html',
        body: `<body style="margin:0;background:#123;color:#fff;font:16px sans-serif;display:grid;place-items:center;height:100vh">PLAYER ${url.pathname.slice(7)}</body>`,
      })
    if (url.hostname === 'www.googleapis.com') {
      const auth = req.headers()['authorization']
      if (req.method() === 'OPTIONS')
        return route.fulfill({
          status: 204,
          headers: {
            'access-control-allow-origin': '*',
            'access-control-allow-headers': '*',
            'access-control-allow-methods': '*',
          },
        })
      const cors = { 'access-control-allow-origin': '*' }
      if (url.pathname.endsWith('/commentThreads') && req.method() === 'POST') {
        const body = req.postDataJSON()
        log.posts.push({ body, auth })
        const s = log.state.post
        if (s === 'quota')
          return route.fulfill({
            status: 403,
            headers: cors,
            json: { error: { errors: [{ reason: 'quotaExceeded' }] } },
          })
        if (s === 'unauthorized')
          return route.fulfill({
            status: 401,
            headers: cors,
            json: { error: { errors: [{ reason: 'authError' }] } },
          })
        const text = body.snippet.topLevelComment.snippet.textOriginal
        return route.fulfill({
          headers: cors,
          json: {
            id: 'thread-' + log.posts.length,
            snippet: {
              videoId: body.snippet.videoId,
              topLevelComment: {
                id: 'mine-' + log.posts.length,
                snippet: {
                  authorDisplayName: '@me',
                  authorProfileImageUrl: 'https://yt3.ggpht.com/me',
                  textOriginal: text,
                  likeCount: 0,
                  publishedAt: new Date().toISOString(),
                },
              },
            },
          },
        })
      }
      if (url.pathname.endsWith('/comments') && req.method() === 'PUT') {
        const body = req.postDataJSON()
        log.puts.push(body)
        return route.fulfill({
          headers: cors,
          json: { id: body.id, snippet: { textOriginal: body.snippet.textOriginal } },
        })
      }
      if (url.pathname.endsWith('/comments') && req.method() === 'DELETE') {
        log.deletes.push(url.searchParams.get('id'))
        return route.fulfill({ status: 204, headers: cors, body: '' })
      }
      return route.fulfill({ status: 500, headers: cors, json: {} })
    }
    if (url.hostname.endsWith('ytimg.com') || url.hostname.endsWith('ggpht.com'))
      return route.fulfill({ contentType: 'image/png', body: PNG })
    return route.abort()
  })
  return { context, page, log }
}

export const noHScroll = (page) =>
  page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)
