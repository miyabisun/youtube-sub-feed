// A page left open outlives the release it came from. Pulling the feed down
// asks the server which release it runs: the same one reloads the list, as
// before; another one loads the whole page again, so the new client replaces
// the old. Nothing but a pull asks.
import { expect, test } from '@playwright/test'
import { BASE, IDS, NEW_ID, check, drag, hosted, setup } from './harness.js'

const GROUPS = [
  { id: 1, name: 'ブルアカ' },
  { id: 2, name: '情報' },
]
const strip = (page) => page.locator('.pull')
const firstCard = (page) =>
  page.locator('.video-card a.thumbnail-link').first().getAttribute('href')
const pull = (page, x = 200) => drag(page, { x, y: 300 }, { x, y: 520 })
// Set once the page is up: gone again only if the document was loaded anew.
const sameDocument = (page) => page.evaluate(() => window.__sameDocument === true)

async function open(browser, { options = {}, host = false, path = '/', version } = {}) {
  const made = await setup(browser, { groups: GROUPS, ...options })
  if (version !== undefined) made.server.version = version
  if (host) await hosted(made.page)
  await made.page.goto(BASE + path)
  await made.page.waitForSelector('.video-card')
  await made.page.evaluate(() => (window.__sameDocument = true))
  return made
}

test('the same release on both sides: a pull reloads the list, not the page', async ({
  browser,
}) => {
  const { context, page, log, server } = await open(browser)
  const [loads, pages] = [log.feed, log.pages]
  check('opening the page did not ask for the version', log.health === 0, String(log.health))

  // The answer about the version waits, then the one with the list.
  let answerHealth, answerFeed
  server.holdHealth = new Promise((resolve) => (answerHealth = resolve))
  server.holdFeed = new Promise((resolve) => (answerFeed = resolve))
  server.newVideo = true
  await pull(page)
  await expect(strip(page)).toHaveText('更新中')
  await expect.poll(() => log.health).toBe(1)
  check('the list waits for the version', log.feed === loads, `${loads} -> ${log.feed}`)

  // A second pull while the version is still out asks nothing more.
  await pull(page)
  await page.waitForTimeout(200)
  check('a second pull meanwhile asks nothing', log.health === 1 && log.feed === loads)

  answerHealth()
  await expect.poll(() => log.feed).toBe(loads + 1)
  answerFeed()
  await expect.poll(() => firstCard(page)).toContain(NEW_ID)
  await expect(strip(page)).toHaveText('')
  check('the page is the same document', await sameDocument(page))
  check('no page was asked for', log.pages === pages, `${pages} -> ${log.pages}`)
  check('the version was asked for once', log.health === 1, String(log.health))
  await context.close()
})

for (const { name, options, host } of [
  { name: 'phone', options: {}, host: false },
  {
    name: 'tablet, inside the host app',
    options: { width: 1024, touch: true, scheme: 'light' },
    host: true,
  },
]) {
  test(`a newer release on the server: a pull loads the whole page again (${name})`, async ({
    browser,
  }) => {
    const { context, page, log, server } = await open(browser, { options, host, path: '/group/2' })
    const x = (options.width ?? 400) / 2
    const [loads, pages] = [log.feed, log.pages]

    // A release goes out while the page stays open.
    server.version = '0.4.5'
    server.newVideo = true
    await page.waitForTimeout(300)
    check('nothing notices until a pull', log.health === 0 && log.pages === pages)

    await pull(page, x)
    await expect.poll(() => log.pages).toBe(pages + 1)
    await page.waitForSelector('.video-card')
    check('the document was loaded anew', !(await sameDocument(page)))
    check(
      'the new page carries the new release',
      (await page.evaluate(() => window.__APP_VERSION__)) === '0.4.5',
    )
    check('it is the same group', page.url() === BASE + '/group/2', page.url())
    check(
      'the list was loaded once, by the new page, for that group',
      log.feed === loads + 1 && log.feedQueries.at(-1).includes('group=2'),
      `${loads} -> ${log.feed} ${log.feedQueries.at(-1)}`,
    )
    await expect.poll(() => firstCard(page)).toContain(NEW_ID)
    await expect(strip(page)).toHaveText('')

    // Now both sides agree: the next pull is a pull again, and loads no page.
    await page.evaluate(() => (window.__sameDocument = true))
    await pull(page, x)
    await expect.poll(() => log.feed).toBe(loads + 2)
    await expect(strip(page)).toHaveText('')
    check('the second pull kept the document', await sameDocument(page))
    check('one page load in all', log.pages === pages + 1, `${pages} -> ${log.pages}`)
    check('one question per pull', log.health === 2, String(log.health))
    check('nothing was opened or marked watched', !log.hide.length && !log.youtube.length)
    await context.close()
  })
}

test('the version cannot be read: a pull reloads the list all the same', async ({ browser }) => {
  const { context, page, log, server } = await open(browser)
  const [loads, pages] = [log.feed, log.pages]
  server.version = '0.4.5'
  server.failHealth = true
  server.newVideo = true
  await pull(page)
  await expect.poll(() => firstCard(page)).toContain(NEW_ID)
  check('it did ask', log.health === 1, String(log.health))
  check('the list was loaded once', log.feed === loads + 1, `${loads} -> ${log.feed}`)
  check('the page is the same document', await sameDocument(page))
  check('no page was asked for', log.pages === pages)
  check('no failure is shown', (await page.locator('.toast.error').count()) === 0)
  await context.close()
})

test('a page that carries no version never reloads itself', async ({ browser }) => {
  // The dev server: its pages come without a version, and a reload would not add one.
  const { context, page, log, server } = await open(browser, { version: '' })
  const [loads, pages] = [log.feed, log.pages]
  server.version = '0.4.5'
  for (const n of [1, 2]) {
    await pull(page)
    await expect.poll(() => log.feed).toBe(loads + n)
    await expect(strip(page)).toHaveText('')
  }
  check('the page is the same document', await sameDocument(page))
  check('no page was asked for', log.pages === pages)
  check('the list is whole', (await page.locator('.video-card').count()) === IDS.length)
  await context.close()
})
