// A video card is a link to YouTube: it opens there in a new tab, the list
// stays as it was, and the video counts as watched.
import { expect, test } from '@playwright/test'
import { BASE, IDS, check, noHScroll, setup } from './harness.js'

const watchUrl = (id) => `https://www.youtube.com/watch?v=${id}`
const card = (page, id) => page.locator(`.video-card:has(a[href*="${id}"])`)

// Do `action` and return the address the new tab asked YouTube for.
async function opened(context, log, action) {
  const seen = log.youtube.length
  const [tab] = await Promise.all([context.waitForEvent('page'), action()])
  await expect.poll(() => log.youtube.length).toBe(seen + 1)
  await tab.close().catch(() => {})
  return log.youtube[seen]
}

test('a tap opens the video on YouTube in a new tab and leaves the list in place', async ({
  browser,
}) => {
  // No GIS client id: lists and links need neither Google nor an API key.
  const { context, page, log } = await setup(browser, { gisClientId: '' })
  await page.goto(BASE + '/')
  await page.waitForSelector('.video-card')
  await page.evaluate(() => {
    window.__sameDocument = true
    document.querySelector('main').scrollTo(0, 420)
  })
  const feedLoads = log.feed
  const links = await card(page, IDS[1])
    .locator('a.thumbnail-link, a.title')
    .evaluateAll((all) => all.map((a) => [a.getAttribute('href'), a.target, a.rel].join(' ')))
  check(
    'thumbnail and title are plain links to the video',
    links.length === 2 && links.every((l) => l === `${watchUrl(IDS[1])} _blank noopener`),
    JSON.stringify(links),
  )

  const url = await opened(context, log, () => card(page, IDS[1]).locator('.thumbnail-link').tap())
  check('the thumbnail opens the video on YouTube', url === watchUrl(IDS[1]), url)
  await card(page, IDS[1]).waitFor({ state: 'detached' })
  check('the tapped video is marked as watched', log.hide.join() === IDS[1], log.hide.join())
  const list = await page.evaluate(() => ({
    same: window.__sameDocument === true,
    scrollTop: document.querySelector('main').scrollTop,
  }))
  check(
    'the list is the same page, still scrolled, and was not reloaded',
    page.url() === BASE + '/' && list.same && list.scrollTop > 0 && log.feed === feedLoads,
    JSON.stringify({ url: page.url(), ...list, feed: [feedLoads, log.feed] }),
  )

  // The Short, the live stream and the stream archive open the same way.
  const short = await opened(context, log, () => card(page, IDS[2]).locator('.title').tap())
  check(
    'the title of a Short opens it at its Shorts address',
    short === `https://www.youtube.com/shorts/${IDS[2]}`,
    short,
  )
  for (const [id, label] of [
    [IDS[3], 'LIVE'],
    [IDS[4], '配信アーカイブ'],
  ]) {
    const shown = await card(page, id).locator('.label').innerText()
    const to = await opened(context, log, () => card(page, id).locator('.thumbnail-link').tap())
    check(`${label} opens on YouTube`, shown === label && to === watchUrl(id), `${shown} ${to}`)
  }
  await page.waitForFunction((n) => document.querySelectorAll('.video-card').length === n, 4)
  check(
    'each opened video is marked once',
    log.hide.join() === IDS.slice(1, 5).join(),
    log.hide.join(),
  )
  check('no page-level horizontal scroll at 400px', await noHScroll(page))
  await context.close()
})

test('new-tab gestures open YouTube and mark the video too', async ({ browser }) => {
  const { context, page, log } = await setup(browser, { width: 1280, height: 800 })
  await page.goto(BASE + '/')
  await page.waitForSelector('.video-card')

  const plain = await opened(context, log, () => card(page, IDS[0]).locator('.title').click())
  check('a click opens a new tab on a desktop', plain === watchUrl(IDS[0]), plain)
  const ctrl = await opened(context, log, () =>
    card(page, IDS[1])
      .locator('.thumbnail-link')
      .click({ modifiers: ['ControlOrMeta'] }),
  )
  check('a Ctrl or Cmd click opens a new tab', ctrl === watchUrl(IDS[1]), ctrl)
  const middle = await opened(context, log, () =>
    card(page, IDS[3]).locator('.thumbnail-link').click({ button: 'middle' }),
  )
  check('a middle click opens a new tab', middle === watchUrl(IDS[3]), middle)
  await page.waitForFunction((n) => document.querySelectorAll('.video-card').length === n, 5)
  check(
    'each of them marks its video as watched',
    log.hide.join() === [IDS[0], IDS[1], IDS[3]].join(),
    log.hide.join(),
  )
  check('the list stays on its page', page.url() === BASE + '/')
  await context.close()
})

test('history, channel and settings pages link to YouTube without marking', async ({ browser }) => {
  const channel = { id: 'UC' + 'x'.repeat(22), title: 'チャンネル', group_names: null }
  const { context, page, log } = await setup(browser, {
    channels: [channel],
    groups: [{ id: 1, name: 'ゲーム', sort_order: 0 }],
  })
  await page.goto(BASE + '/history')
  const fromHistory = await opened(context, log, () =>
    page.locator('.video-card').first().locator('.thumbnail-link').tap(),
  )
  check('history opens the video on YouTube', fromHistory === watchUrl(IDS[0]), fromHistory)
  check(
    'history keeps its list',
    page.url() === BASE + '/history' && (await page.locator('.video-card').count()) === 2,
  )

  await page.goto(BASE + '/channel/' + channel.id)
  const fromChannel = await opened(context, log, () => card(page, IDS[2]).locator('.title').tap())
  check(
    'a channel page opens a Short at its Shorts address',
    fromChannel === `https://www.youtube.com/shorts/${IDS[2]}`,
    fromChannel,
  )

  await page.goto(BASE + '/settings')
  await page.getByRole('button', { name: '割当' }).tap()
  await page.locator('.assign-row .channel-name').tap()
  const fromSettings = await opened(context, log, () => page.locator('.video-thumb').nth(1).tap())
  check('a settings thumbnail opens on YouTube', fromSettings === watchUrl(IDS[1]), fromSettings)
  check('none of these pages marks a video', log.hide.length === 0, log.hide.join())
  await context.close()
})

// Addresses of the watch view that was removed can linger in history and bookmarks.
test('an old watch address shows the list behind it', async ({ browser }) => {
  const { context, page } = await setup(browser)
  await page.goto(BASE + '/?watch=' + IDS[3])
  await page.waitForSelector('.video-card')
  check(
    'the feed shows with nothing over it',
    (await page.locator('.video-card').count()) === IDS.length &&
      (await page.locator('dialog[open], iframe').count()) === 0 &&
      context.pages().length === 1,
  )
  await page.goto(BASE + '/channels')
  await page.waitForSelector('.menu-button')
  await page.goBack()
  await page.waitForSelector('.video-card')
  check(
    'going back to it shows the feed again',
    (await page.locator('.video-card').count()) === IDS.length,
  )
  await page.goto(BASE + '/history?watch=' + IDS[0])
  await page.waitForSelector('.video-card')
  check('on history it shows history', (await page.locator('.video-card').count()) === 2)
  await context.close()
})
