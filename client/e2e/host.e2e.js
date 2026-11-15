// Inside the app that hosts this page (subfeed-browser), opening a video is
// the app's job: the page marks it watched and hands the video over.
import { test } from '@playwright/test'
import { BASE, IDS, check, setup } from './harness.js'

// What the host does to every page before its scripts run, plus a recorder.
async function hosted(page) {
  await page.addInitScript(() => {
    window.__handedOver = []
    window.addEventListener('message', (event) => {
      if (event.source === window && event.data?.source === 'youtube-sub-feed')
        window.__handedOver.push(event.data)
    })
    const mark = () => {
      if (!document.documentElement) return false
      document.documentElement.dataset.videoHost = '1'
      return true
    }
    if (!mark())
      new MutationObserver((_, observer) => mark() && observer.disconnect()).observe(document, {
        childList: true,
      })
  })
}
const handedOver = (page) => page.evaluate(() => window.__handedOver)
const until = (page, count) => page.waitForFunction((n) => window.__handedOver.length === n, count)

test('in the app a tap hands the video over instead of playing it here', async ({ browser }) => {
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
    'nothing plays in the page',
    (await page.locator('dialog.watch').count()) === 0 &&
      !page.url().includes('watch=') &&
      context.pages().length === 1,
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
    (await handedOver(page))[0].id === IDS[0] && (await page.locator('dialog.watch').count()) === 0,
  )
  await context.close()
})

test('in the app a watch address still plays here', async ({ browser }) => {
  const { context, page } = await setup(browser)
  await hosted(page)
  await page.goto(BASE + '/?watch=' + IDS[3])
  await page.waitForSelector('dialog.watch[open]')
  check('a loaded watch address hands nothing over', (await handedOver(page)).length === 0)
  await context.close()
})
