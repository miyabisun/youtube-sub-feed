// The feed reloads when it is pulled down from its top, and at no other time
// on its own: not on a timer, not on coming back to the page.
import { expect, test } from '@playwright/test'
import { BASE, IDS, NEW_ID, check, drag, hosted, noHScroll, setup } from './harness.js'

const strip = (page) => page.locator('.pull')
const stripHeight = (page) => strip(page).evaluate((el) => el.getBoundingClientRect().height)
const firstCard = (page) =>
  page.locator('.video-card a.thumbnail-link').first().getAttribute('href')
const scrollTop = (page) => page.evaluate(() => document.querySelector('main').scrollTop)

async function open(browser, options = {}, { host = false, path = '/' } = {}) {
  const made = await setup(browser, options)
  if (host) await hosted(made.page)
  await made.page.goto(BASE + path)
  await made.page.waitForSelector('.video-card')
  // Whether the page took each move for itself, and whether it is still the
  // same document afterwards (the browser's own pull would load a new one).
  await made.page.evaluate(() => {
    window.__sameDocument = true
    window.__moves = []
    window.addEventListener('touchmove', (e) => window.__moves.push(e.defaultPrevented))
  })
  return made
}

for (const { name, options, host } of [
  { name: 'phone, dark', options: {}, host: false },
  { name: 'phone, light, inside the host app', options: { scheme: 'light' }, host: true },
  { name: 'tablet, dark', options: { width: 1024, touch: true }, host: false },
  {
    name: 'tablet, light, inside the host app',
    options: { width: 1024, touch: true, scheme: 'light' },
    host: true,
  },
]) {
  test(`pulling the feed down past the threshold reloads it (${name})`, async ({ browser }) => {
    const { context, page, log, server } = await open(browser, options, { host })
    const x = (options.width ?? 400) / 2
    const loads = log.feed
    check('nothing shows before a pull', (await stripHeight(page)) === 0)

    // The next answer has a new video on top, and waits until the spec lets it go.
    let answer
    server.holdFeed = new Promise((resolve) => (answer = resolve))
    server.newVideo = true

    // A little way down: the strip opens and says what a pull does.
    const lift = await drag(page, { x, y: 300 }, { x, y: 360 }, { release: false })
    await expect(strip(page)).toHaveText('引いて更新')
    const shallow = await stripHeight(page)
    check('the strip opens as the finger moves', shallow > 0, String(shallow))
    await lift()
    await expect.poll(() => stripHeight(page)).toBe(0)
    check('a short pull loads nothing', log.feed === loads, `${loads} -> ${log.feed}`)

    // Past the threshold: it says so, and letting go loads.
    const release = await drag(page, { x, y: 300 }, { x, y: 520 }, { release: false })
    await expect(strip(page)).toHaveText('離して更新')
    check('still nothing is loaded while the finger is down', log.feed === loads)
    await release()
    await expect(strip(page)).toHaveText('更新中')
    await expect.poll(() => log.feed).toBe(loads + 1)
    check(
      'it asks for the list from its start',
      new URLSearchParams(log.feedQueries.at(-1)).get('offset') === '0',
      log.feedQueries.at(-1),
    )
    check(
      'the list stays on screen while it loads',
      (await page.locator('.video-card').count()) > 0,
    )

    // Pulling again while that load is still out starts no second one.
    await drag(page, { x, y: 300 }, { x, y: 520 })
    await page.waitForTimeout(200)
    check('a second pull during the load loads nothing more', log.feed === loads + 1, `${log.feed}`)
    await expect(strip(page)).toHaveText('更新中')

    answer()
    await expect.poll(() => firstCard(page)).toContain(NEW_ID)
    await expect.poll(() => stripHeight(page)).toBe(0)
    await expect(strip(page)).toHaveText('')
    check(
      'the list was replaced, not appended to',
      (await page.locator('.video-card').count()) === IDS.length + 1,
    )

    const after = await page.evaluate(() => ({
      same: window.__sameDocument === true,
      taken: window.__moves.length > 0 && window.__moves.every(Boolean),
      contained: getComputedStyle(document.querySelector('main')).overscrollBehaviorY,
    }))
    check(
      "the browser's own pull-to-refresh gets no part of the gesture",
      after.same && after.taken && after.contained === 'contain',
      JSON.stringify(after),
    )
    check(
      'the pulls opened no video and marked none watched',
      log.hide.length === 0 && log.youtube.length === 0 && context.pages().length === 1,
      JSON.stringify([log.hide, log.youtube]),
    )
    check('the page did not leave the feed', page.url() === BASE + '/')
    check('no sideways scroll', await noHScroll(page))
    await context.close()
  })
}

test('away from the top a downward drag scrolls and reloads nothing', async ({ browser }) => {
  const { context, page, log } = await open(browser)
  const loads = log.feed
  await page.evaluate(() => document.querySelector('main').scrollTo(0, 420))
  await drag(page, { x: 200, y: 300 }, { x: 200, y: 520 })
  await expect.poll(() => scrollTop(page)).toBeLessThan(420)
  check('the list scrolled back up, short of its top', (await scrollTop(page)) > 0)
  check('the strip stayed shut', (await stripHeight(page)) === 0)
  await page.waitForTimeout(200)
  check('nothing was loaded', log.feed === loads, `${loads} -> ${log.feed}`)
  await context.close()
})

test('a reload that fails keeps the list and says so', async ({ browser }) => {
  const { context, page, log, server } = await open(browser)
  const loads = log.feed
  server.failFeed = true
  await drag(page, { x: 200, y: 300 }, { x: 200, y: 520 })
  await expect(page.locator('.toast.error')).toBeVisible()
  check('it did try', log.feed === loads + 1)
  check('the list is still there', (await page.locator('.video-card').count()) === IDS.length)
  await expect.poll(() => stripHeight(page)).toBe(0)

  // And the next pull works again.
  server.failFeed = false
  server.newVideo = true
  await drag(page, { x: 200, y: 300 }, { x: 200, y: 520 })
  await expect.poll(() => firstCard(page)).toContain(NEW_ID)
  await context.close()
})

test('left alone, the feed loads nothing: not after minutes, not on coming back', async ({
  browser,
}) => {
  const { context, page, log } = await setup(browser)
  await page.clock.install()
  await page.goto(BASE + '/')
  await page.waitForSelector('.video-card')
  const loads = log.feed

  await page.clock.runFor('02:30')
  // Another tab or app comes to the front, then this page again.
  await page.evaluate(() => {
    const show = (state) => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state })
      Object.defineProperty(document, 'hidden', {
        configurable: true,
        get: () => state === 'hidden',
      })
      document.dispatchEvent(new Event('visibilitychange'))
    }
    show('hidden')
    window.dispatchEvent(new Event('blur'))
    window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true }))
    show('visible')
    window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true }))
    window.dispatchEvent(new Event('focus'))
  })
  await page.clock.runFor('02:30')
  await page.waitForTimeout(300)
  check('the feed was asked for once, at the start', log.feed === loads, `${loads} -> ${log.feed}`)
  await context.close()
})

test('swipes, taps and a held finger keep their own jobs beside the pull', async ({ browser }) => {
  const { context, page, log, server } = await open(browser, {
    groups: [
      { id: 1, name: 'ブルアカ' },
      { id: 2, name: '情報' },
    ],
  })
  let loads = log.feed

  // Sideways: the next group, and the load that goes with it.
  await drag(page, { x: 320, y: 400 }, { x: 120, y: 410 })
  await expect.poll(() => page.url()).toBe(BASE + '/group/1')
  await expect.poll(() => log.feed).toBe(loads + 1)
  check('the group asked for is in the request', log.feedQueries.at(-1).includes('group=1'))
  await expect(page.locator('.video-card')).toHaveCount(IDS.length)
  loads = log.feed

  // A diagonal that is more sideways than down switches the group and pulls nothing.
  const watch = strip(page).evaluate(
    (el) =>
      new Promise((resolve) => {
        let tallest = 0
        const observer = new ResizeObserver(() => {
          tallest = Math.max(tallest, el.getBoundingClientRect().height)
        })
        observer.observe(el)
        setTimeout(() => resolve(tallest), 900)
      }),
  )
  await drag(page, { x: 340, y: 300 }, { x: 140, y: 480 })
  await expect.poll(() => page.url()).toBe(BASE + '/group/2')
  check('the strip never opened during that swipe', (await watch) === 0)
  await page.waitForTimeout(200)
  check('one load, for the group', log.feed === loads + 1, `${loads} -> ${log.feed}`)
  await expect(page.locator('.video-card')).toHaveCount(IDS.length)
  loads = log.feed

  // A diagonal that is more down than sideways pulls and leaves the group alone.
  server.newVideo = true
  await drag(page, { x: 140, y: 300 }, { x: 240, y: 540 })
  await expect.poll(() => firstCard(page)).toContain(NEW_ID)
  check('the group did not change', page.url() === BASE + '/group/2', page.url())
  check('one load, for the pull', log.feed === loads + 1, `${loads} -> ${log.feed}`)
  check('that load kept the group', log.feedQueries.at(-1).includes('group=2'))
  loads = log.feed

  // A finger held still, trembling a little, is neither: the page takes none
  // of it, so the browser's own long press is free to open. (Headless Chromium
  // opens no menu, so the touch ends here the way a menu would end it.)
  const before = await page.evaluate(() => window.__moves.length)
  const lift = await drag(page, { x: 200, y: 300 }, { x: 203, y: 303 }, { release: false })
  await page.waitForTimeout(700)
  check('holding still opens no strip', (await stripHeight(page)) === 0)
  const held = await page.evaluate((n) => window.__moves.slice(n), before)
  check('the page took none of the held touch', !held.some(Boolean), JSON.stringify(held))
  await lift({ cancel: true })
  await page.waitForTimeout(200)
  check('and loads nothing', log.feed === loads, `${loads} -> ${log.feed}`)

  // After the group switches the list still keeps the browser's overscroll in;
  // the pages without the pull are as they were.
  const contained = () =>
    page.evaluate(() => getComputedStyle(document.querySelector('main')).overscrollBehaviorY)
  check('the feed of a group contains overscroll', (await contained()) === 'contain')

  // A tap is still a tap.
  const cards = await page.locator('.video-card').count()
  await page.locator('.hide-btn').first().tap()
  await expect.poll(() => page.locator('.video-card').count()).toBe(cards - 1)
  check('「もう見た」 marks the tapped video', log.hide.length === 1, log.hide.join())

  await page.goto(BASE + '/history')
  await page.waitForSelector('.video-card')
  check('a page without the pull leaves overscroll alone', (await contained()) === 'auto')
  check('and has no strip', (await strip(page).count()) === 0)
  await context.close()
})
