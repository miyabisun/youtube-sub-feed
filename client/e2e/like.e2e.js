// Liking the open video, against the stand-ins of ./harness.js: no rating
// ever reaches YouTube.
import { test } from '@playwright/test'
import { BASE, IDS, check, setup } from './harness.js'

const like = (page) => page.getByRole('button', { name: '高評価' })
const pressed = async (page) => (await like(page).getAttribute('aria-pressed')) === 'true'
const untilPressed = (page, value) =>
  page.waitForSelector(`dialog.watch .like[aria-pressed="${value}"]:not(:disabled)`)
const gisRequests = (page) => page.evaluate(() => window.__gis?.scopes.length ?? 0)

test('a like goes to the open video, comes off again, and is remembered', async ({ browser }) => {
  const { context, page, log } = await setup(browser, { ratings: { [IDS[2]]: 'like' } })
  await page.goto(BASE + '/')
  await page.locator('.video-card').nth(1).locator('.thumbnail-link').tap()
  await page.waitForSelector('dialog.watch[open]')
  await page.waitForSelector('.comment')
  check('the like button starts unpressed', !(await pressed(page)))
  check(
    'opening a video asks Google for nothing',
    (await gisRequests(page)) === 0 && log.ratingGets.length === 0,
  )

  await like(page).tap()
  await untilPressed(page, true)
  check(
    'the like is addressed to the open video with the viewer token',
    log.rates.length === 1 &&
      log.rates[0].id === IDS[1] &&
      log.rates[0].rating === 'like' &&
      log.rates[0].auth === 'Bearer TOKEN',
    JSON.stringify(log.rates),
  )
  // The finger is still on the button: a touch screen keeps it hovered.
  const look = await like(page).evaluate((el) => {
    const probe = document.body.appendChild(document.createElement('i'))
    probe.style.color = 'var(--c-accent)'
    const accent = getComputedStyle(probe).color
    probe.remove()
    return {
      accent: getComputedStyle(el).color === accent,
      filled: getComputedStyle(el.querySelector('path')).fill !== 'none',
    }
  })
  check('a like shows as a filled accent thumb', look.accent && look.filled, JSON.stringify(look))
  check(
    'the write scope is requested when liking',
    (await page.evaluate(() => window.__gis.scopes)).join() ===
      'https://www.googleapis.com/auth/youtube.force-ssl',
  )

  await like(page).tap()
  await untilPressed(page, false)
  check(
    'a second tap takes the like off',
    log.rates[1].id === IDS[1] && log.rates[1].rating === 'none',
    JSON.stringify(log.rates[1]),
  )

  // The comment after a like needs no second authorization.
  await page.getByRole('textbox', { name: 'コメント', exact: true }).fill('よかった')
  await page.getByRole('button', { name: '送信' }).tap()
  await page.waitForSelector('.comment.own')
  check('like and comment share one authorization', (await gisRequests(page)) === 1)

  await like(page).tap()
  await untilPressed(page, true)
  await page.goBack()
  await page.waitForSelector('dialog.watch', { state: 'detached' })

  // Another video, liked on an earlier day: authorized now, so its state is read.
  await page.locator('.video-card').nth(1).locator('.title').tap()
  await page.waitForSelector('dialog.watch[open]')
  check('a second video opens', new URL(page.url()).searchParams.get('watch') === IDS[2])
  await untilPressed(page, true)
  check(
    'an earlier like is read for the open video',
    log.ratingGets.length === 1 &&
      log.ratingGets[0].id === IDS[2] &&
      log.ratingGets[0].auth === 'Bearer TOKEN',
    JSON.stringify(log.ratingGets),
  )
  await like(page).tap()
  await untilPressed(page, false)
  check(
    'the like comes off the second video, not the first',
    log.rates.at(-1).id === IDS[2] && log.rates.at(-1).rating === 'none',
    JSON.stringify(log.rates.at(-1)),
  )
  check('the whole visit asked Google once', (await gisRequests(page)) === 1)

  await page.goBack()
  await page.waitForSelector('dialog.watch', { state: 'detached' })
  await page.goto(BASE + '/?watch=' + IDS[1])
  await page.waitForSelector('dialog.watch[open]')
  check('a video opened by its address has the like button', (await like(page).count()) === 1)
  await context.close()
})

test('a like that fails says why and stays as it was', async ({ browser }) => {
  const { context, page, log } = await setup(browser, { gis: 'cancel' })
  await page.goto(BASE + '/history')
  await page.locator('.video-card').first().locator('.thumbnail-link').tap()
  await page.waitForSelector('dialog.watch[open]')
  const notice = page.locator('dialog.watch .like-notice')

  await like(page).tap()
  await notice.waitFor()
  check(
    'cancelled authorization is explained and nothing is sent',
    (await notice.innerText()) === 'Google の認可がキャンセルされました' &&
      log.rates.length === 0 &&
      !(await pressed(page)),
    await notice.innerText(),
  )

  await page.evaluate(() => (window.__gis.mode = 'ok'))
  log.state.rate = 'quota'
  await like(page).tap()
  await page.waitForFunction(() =>
    document.querySelector('dialog.watch .like-notice')?.innerText.includes('割り当て'),
  )
  check('an exhausted quota leaves the button unpressed', !(await pressed(page)))
  check(
    'a failure uses the danger role',
    await notice.evaluate((el) => el.classList.contains('error')),
  )

  log.state.rate = 'unauthorized'
  await like(page).tap()
  await page.waitForFunction(() =>
    document.querySelector('dialog.watch .like-notice')?.innerText.includes('認可が切れました'),
  )

  log.state.rate = 'ok'
  await like(page).tap()
  await untilPressed(page, true)
  check(
    'the retry goes through and the notice leaves',
    (await notice.count()) === 0 && log.rates.at(-1).rating === 'like',
  )
  check('a rejected token is requested again', (await gisRequests(page)) === 3)
  await context.close()
})

test('a video with ratings turned off says so without an error', async ({ browser }) => {
  const { context, page } = await setup(browser, { rate: 'disabled' })
  await page.goto(BASE + '/?watch=' + IDS[0])
  await page.waitForSelector('dialog.watch[open]')
  await like(page).tap()
  const notice = page.locator('dialog.watch .like-notice')
  await notice.waitFor()
  check(
    'ratings turned off: a muted note, not a failure',
    (await notice.innerText()) === 'この動画は評価を受け付けていません' &&
      !(await notice.evaluate((el) => el.classList.contains('error'))),
  )
  check(
    'ratings turned off: the button is unpressed and disabled',
    !(await pressed(page)) && (await like(page).isDisabled()),
  )
  await context.close()
})

test('without a GIS client id there is no like button', async ({ browser }) => {
  const { context, page } = await setup(browser, { gisClientId: '' })
  await page.goto(BASE + '/?watch=' + IDS[3])
  await page.waitForSelector('.comment')
  check('no GIS client id: no like button', (await like(page).count()) === 0)
  await context.close()
})

test('without an API key the like still works', async ({ browser }) => {
  const { context, page, log } = await setup(browser, { comments: 'unavailable' })
  await page.goto(BASE + '/?watch=' + IDS[3])
  await page.waitForSelector('dialog.watch[open]')
  await like(page).tap()
  await untilPressed(page, true)
  check('no API key: the like is sent', log.rates[0]?.id === IDS[3])
  await context.close()
})
