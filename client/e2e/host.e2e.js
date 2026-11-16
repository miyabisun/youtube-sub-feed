// Inside the app that hosts this page (subfeed-browser), opening a video is
// the app's job: the page marks it watched and hands the video over.
import { test } from '@playwright/test'
import { BASE, IDS, check, hosted, setup } from './harness.js'

const handedOver = (page) => page.evaluate(() => window.__handedOver)
const until = (page, count) => page.waitForFunction((n) => window.__handedOver.length === n, count)

test('in the app a tap hands the video over instead of opening a tab', async ({ browser }) => {
  const { context, page, log } = await setup(browser)
  await hosted(page)
  await page.goto(BASE + '/')
  await page.waitForSelector('.video-card')
  const feedLoads = log.feed

  await page.locator('.video-card').nth(1).locator('.thumbnail-link').tap()
  await until(page, 1)
  await page.waitForFunction(
    (n) => document.querySelectorAll('.video-card').length === n,
    IDS.length - 1,
  )
  let sent = await handedOver(page)
  check(
    'the handed-over video is the tapped one',
    sent[0].type === 'open-video' &&
      sent[0].id === IDS[1] &&
      sent[0].url === `https://www.youtube.com/watch?v=${IDS[1]}`,
    JSON.stringify(sent),
  )
  check(
    'the page stays where it is and opens no tab',
    page.url() === BASE + '/' && context.pages().length === 1,
    page.url(),
  )
  check('the video is still marked as watched', log.hide.join() === IDS[1], log.hide.join())
  check('the list is not reloaded', log.feed === feedLoads, `${feedLoads} -> ${log.feed}`)

  // The next card is the Short; its title link hands over as well.
  await page.locator('.video-card').nth(1).locator('.title').tap()
  await until(page, 2)
  sent = await handedOver(page)
  check(
    'a second tap hands over the second video, a Short at its Shorts address',
    sent[1].id === IDS[2] && sent[1].url === `https://www.youtube.com/shorts/${IDS[2]}`,
    JSON.stringify(sent[1]),
  )
  await context.close()
})

test('in the app history and channel pages hand over too', async ({ browser }) => {
  const channel = { id: 'UC' + 'x'.repeat(22), title: 'チャンネル', group_names: null }
  const { context, page, log } = await setup(browser, { channels: [channel] })
  await hosted(page)
  await page.goto(BASE + '/history')
  await page.locator('.video-card').first().locator('.thumbnail-link').tap()
  await until(page, 1)
  check('history hands over', (await handedOver(page))[0].id === IDS[0])
  check('opening from history marks nothing', log.hide.length === 0)

  await page.goto(BASE + '/channel/' + channel.id)
  await page.locator('.video-card').first().locator('.thumbnail-link').tap()
  await until(page, 1)
  check(
    'a channel page hands over',
    (await handedOver(page))[0].id === IDS[0] && context.pages().length === 1,
  )
  await context.close()
})

// The host keeps one part of the page on screen above YouTube: the element
// marked `data-video-host-keep`. Here that is the header, wherever it shows.
test('the header is the one part marked for the host to keep on screen', async ({ browser }) => {
  for (const width of [400, 1024]) {
    const { context, page } = await setup(browser, {
      width,
      groups: [
        { id: 1, name: 'ブルアカ' },
        { id: 2, name: '情報' },
      ],
    })
    for (const path of ['/', '/group/1', '/group/2', '/history', '/channels', '/settings']) {
      await page.goto(BASE + path)
      await page.waitForSelector('.menu-button')
      const kept = await page.evaluate(() =>
        [...document.querySelectorAll('[data-video-host-keep]')].map((el) => {
          const shown = (selector) => !!el.querySelector(selector)?.getClientRects().length
          const box = el.getBoundingClientRect()
          return {
            groups: shown('.nav-tabs') || shown('.group-select'),
            menu: shown('.menu-button'),
            atTop: box.top === 0 && box.left === 0 && box.width === window.innerWidth,
          }
        }),
      )
      check(
        `${width}px ${path}: one marked element at the top edge holds the group switch and the menu`,
        kept.length === 1 && kept[0].groups && kept[0].menu && kept[0].atTop,
        JSON.stringify(kept),
      )
    }
    await page.goto(BASE + '/login')
    await page.waitForSelector('main')
    check(
      `${width}px: a page without the header marks nothing`,
      (await page.locator('header').count()) === 0 &&
        (await page.locator('[data-video-host-keep]').count()) === 0,
    )
    await context.close()
  }
})
