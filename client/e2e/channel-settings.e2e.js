// The per-channel switches: 投稿NG sits beside ショートNG, saves on its own,
// and fits a phone without pushing the page sideways.
import { expect, test } from '@playwright/test'
import { BASE, check, noHScroll, setup } from './harness.js'

const ID = 'UC' + 'x'.repeat(22)
const channel = (settings = {}) => ({
  id: ID,
  title: 'とても長いチャンネルの名前 とても長いチャンネルの名前',
  group_names: null,
  is_favorite: 0,
  show_livestreams: 0,
  hide_shorts: 0,
  hide_posts: 0,
  ...settings,
})
const toggle = (page, name) => page.getByRole('switch', { name })
const open = async (page) => {
  await page.goto(BASE + '/channel/' + ID)
  await page.waitForSelector('[role=switch]')
}

test('投稿NG saves by itself and survives a reload', async ({ browser }) => {
  const { context, page, log } = await setup(browser, { channels: [channel({ hide_shorts: 1 })] })
  await open(page)
  await expect.soft(toggle(page, '投稿NG')).toHaveAttribute('aria-checked', 'false')
  await expect.soft(toggle(page, 'ショートNG')).toHaveAttribute('aria-checked', 'true')
  await expect
    .soft(toggle(page, '投稿NG'))
    .toHaveAccessibleDescription('ONにすると、このチャンネルの投稿を動画一覧から除外します。')
  await expect
    .soft(toggle(page, 'ショートNG'))
    .toHaveAccessibleDescription(
      'ONにすると、このチャンネルのShortsを動画一覧・RSS・新着から除外します。',
    )

  await toggle(page, '投稿NG').tap()
  await expect(toggle(page, '投稿NG')).toHaveAttribute('aria-checked', 'true')
  await expect.soft(page.getByText('設定を更新しました')).toBeVisible()
  check(
    'only hide_posts is sent',
    JSON.stringify(log.settings) === JSON.stringify([{ id: ID, body: { hide_posts: 1 } }]),
    JSON.stringify(log.settings),
  )
  await expect.soft(toggle(page, 'ショートNG')).toHaveAttribute('aria-checked', 'true')

  await page.reload()
  await page.waitForSelector('[role=switch]')
  await expect.soft(toggle(page, '投稿NG')).toHaveAttribute('aria-checked', 'true')

  await toggle(page, '投稿NG').tap()
  await expect(toggle(page, '投稿NG')).toHaveAttribute('aria-checked', 'false')
  check(
    'turning it off sends 0',
    JSON.stringify(log.settings[1]?.body) === JSON.stringify({ hide_posts: 0 }),
    JSON.stringify(log.settings),
  )
  await context.close()
})

test('a refused save leaves 投稿NG off and reports the failure', async ({ browser }) => {
  const { context, page, log } = await setup(browser, {
    channels: [channel()],
    failSettings: true,
  })
  await open(page)
  await toggle(page, '投稿NG').tap()
  await expect(page.getByText(/^500/)).toBeVisible()
  await expect.soft(toggle(page, '投稿NG')).toHaveAttribute('aria-checked', 'false')
  check('the save was attempted once', log.settings.length === 1, JSON.stringify(log.settings))
  await context.close()
})

test('投稿NG is reachable and operable by keyboard', async ({ browser }) => {
  const { context, page, log } = await setup(browser, { width: 1280, channels: [channel()] })
  await open(page)
  await toggle(page, 'ショートNG').focus()
  await page.keyboard.press('Tab')
  await expect(toggle(page, '投稿NG')).toBeFocused()
  const ring = await toggle(page, '投稿NG').evaluate((el) => {
    const style = getComputedStyle(el)
    return style.outlineStyle === 'none' ? 0 : parseFloat(style.outlineWidth)
  })
  check('the focused switch shows the ring', ring > 0, String(ring))
  await page.keyboard.press('Space')
  await expect(toggle(page, '投稿NG')).toHaveAttribute('aria-checked', 'true')
  check(
    'the key press saved it',
    JSON.stringify(log.settings[0]?.body) === JSON.stringify({ hide_posts: 1 }),
    JSON.stringify(log.settings),
  )
  await context.close()
})

for (const [width, scheme] of [
  [320, 'dark'],
  [400, 'dark'],
  [400, 'light'],
  [1280, 'dark'],
]) {
  test(`the four switches fit a ${width}px ${scheme} page`, async ({ browser }) => {
    const { context, page } = await setup(browser, {
      width,
      scheme,
      channels: [channel({ hide_posts: 1 })],
    })
    await open(page)
    check('the page does not scroll sideways', await noHScroll(page))
    const boxes = await page.evaluate(() =>
      [...document.querySelectorAll('[role=switch]')].map((el) => {
        const r = el.getBoundingClientRect()
        const track = el.querySelector('.toggle-track').getBoundingClientRect()
        return {
          name: el.innerText.trim(),
          left: r.left,
          right: r.right,
          top: r.top,
          bottom: r.bottom,
          oneLine: r.height <= track.height + 1,
        }
      }),
    )
    check(
      'the switches are the four settings in order',
      boxes.map((b) => b.name).join() === 'お気に入り,ライブ表示,ショートNG,投稿NG',
      boxes.map((b) => b.name).join(),
    )
    check(
      'each switch is inside the page with its label on one line',
      boxes.every((b) => b.left >= 0 && b.right <= width && b.oneLine),
      JSON.stringify(boxes),
    )
    const overlap = boxes.some((a, i) =>
      boxes.some(
        (b, j) =>
          i < j && a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom,
      ),
    )
    check('no switch covers another', !overlap, JSON.stringify(boxes))
    // On: the track takes the accent colour and the thumb sits at the right.
    const on = await toggle(page, '投稿NG').evaluate((el) => {
      const probe = document.body.appendChild(document.createElement('i'))
      probe.style.color = 'var(--c-accent)'
      const accent = getComputedStyle(probe).color
      probe.remove()
      const track = el.querySelector('.toggle-track')
      const thumb = el.querySelector('.toggle-thumb').getBoundingClientRect()
      const t = track.getBoundingClientRect()
      return {
        accent: getComputedStyle(track).backgroundColor === accent,
        thumbRight: thumb.left - t.left > t.right - thumb.right,
      }
    })
    check('an ON switch is drawn as on', on.accent && on.thumbRight, JSON.stringify(on))
    await context.close()
  })
}
