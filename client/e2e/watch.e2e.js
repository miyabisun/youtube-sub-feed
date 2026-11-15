// The watch view against the stand-ins of ./harness.js: nothing leaves the
// machine and no comment is ever posted.
import { test } from '@playwright/test'
import { BASE, IDS, check, noHScroll, setup } from './harness.js'

// Landscape layout: where the video, the back button and the comment box are.
const sideBySide = (page) =>
  page.evaluate(() => {
    const rect = (selector) => {
      const r = document.querySelector(selector).getBoundingClientRect()
      return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width }
    }
    const inView = (r) =>
      r.left >= 0 && r.top >= 0 && r.right <= innerWidth + 1 && r.bottom <= innerHeight + 1
    const player = rect('dialog.watch .player'),
      box = rect('dialog.watch textarea'),
      back = rect('dialog.watch .bar .icon-btn'),
      send = rect('dialog.watch .compose .primary')
    return {
      ok: inView(player) && inView(box) && inView(back) && inView(send) && box.left >= player.right,
      player,
      box,
      iw: innerWidth,
      ih: innerHeight,
    }
  })

test('a video plays in the app, takes a comment for itself, and closes back to the list', async ({
  browser,
}) => {
  const { context, page, log } = await setup(browser)
  await page.goto(BASE + '/')
  await page.waitForSelector('.video-card')
  await page.evaluate(() => document.querySelector('main').scrollTo(0, 420))
  // The tapped card leaves the list, so "the same place" is measured on its neighbour.
  const nextTop = () =>
    page.evaluate(
      (id) =>
        [...document.querySelectorAll('.video-card')]
          .find((c) => c.querySelector('a').href.includes(id))
          .getBoundingClientRect().top,
      IDS[2],
    )
  const before = await nextTop()
  const feedLoads = log.feed
  await page.locator('.video-card').nth(1).locator('.thumbnail-link').tap()
  await page.waitForSelector('dialog.watch[open]')
  check('tap opens the watch view in the same page', context.pages().length === 1)
  check(
    'url carries the video id',
    new URL(page.url()).searchParams.get('watch') === IDS[1],
    page.url(),
  )
  const src = await page.locator('dialog.watch iframe').getAttribute('src')
  check(
    'player embeds the tapped video',
    src.startsWith(`https://www.youtube.com/embed/${IDS[1]}?`) &&
      src.includes('playsinline=1') &&
      src.includes('origin='),
    src,
  )
  await page.waitForFunction(() => window.__players?.length === 1)
  check('tap records the video as watched', log.hide.join() === IDS[1], log.hide.join())
  await page.waitForSelector('.comment')
  check('comments of that video are requested', log.commentGets[0].id === IDS[1])
  check(
    'comment text is rendered as text, not html',
    (await page.locator('.comment b').count()) === 0 &&
      (await page.locator('.comment').first().innerText()).includes('<b>not html</b>'),
  )
  check('no page-level horizontal scroll at 400px', await noHScroll(page))

  // sticky player while comments scroll
  await page.evaluate(() => document.querySelector('dialog.watch').scrollTo(0, 600))
  const playerTop = await page.evaluate(
    () => document.querySelector('dialog.watch .player').getBoundingClientRect().top,
  )
  check(
    'player stays on screen while comments scroll (phone)',
    Math.abs(playerTop - 48) <= 1,
    `top=${playerTop}`,
  )

  // long comment clamps and expands
  check(
    'long comment is clamped with an expand link',
    (await page.locator('.comment .more').count()) === 1,
  )

  // more
  await page.getByRole('button', { name: 'もっと読む' }).tap()
  await page.waitForFunction(() => document.querySelectorAll('.comment').length === 6)
  check('next page uses the continuation token', log.commentGets[1].token === 'P2')
  check(
    'no more button on the last page',
    (await page.getByRole('button', { name: 'もっと読む' }).count()) === 0,
  )

  // comment
  await page.evaluate(() => document.querySelector('dialog.watch').scrollTo(0, 0))
  const box = page.getByRole('textbox', { name: 'コメント', exact: true })
  const h0 = (await box.boundingBox()).height
  await box.fill('1行目\n2行目\n3行目\n4行目\n5行目\n6行目\n7行目\n8行目')
  const h1 = (await box.boundingBox()).height
  check('comment box grows with long text', h1 > h0 + 40, `${h0} -> ${h1}`)
  await page.getByRole('button', { name: '送信' }).tap()
  await page.waitForSelector('.comment.own')
  check(
    'comment is addressed to the open video',
    log.posts[0].body.snippet.videoId === IDS[1],
    JSON.stringify(log.posts[0].body.snippet.videoId),
  )
  check(
    'comment keeps its line breaks',
    log.posts[0].body.snippet.topLevelComment.snippet.textOriginal.split('\n').length === 8,
  )
  check('request carries the viewer token', log.posts[0].auth === 'Bearer TOKEN')
  check(
    'write scope is requested only when sending',
    (await page.evaluate(() => window.__gis.scopes)).join() ===
      'https://www.googleapis.com/auth/youtube.force-ssl',
  )
  check(
    'posted comment is shown and the box is emptied',
    (await page.locator('.comment.own').innerText()).includes('1行目') &&
      (await box.inputValue()) === '',
  )

  // edit
  await page.getByRole('button', { name: '編集' }).tap()
  await page.getByRole('textbox', { name: 'コメントを編集' }).fill('直しました')
  await page.getByRole('button', { name: '保存' }).tap()
  await page.waitForFunction(() =>
    document.querySelector('.comment.own')?.innerText.includes('直しました'),
  )
  check(
    'edit updates that comment',
    log.puts[0].id === 'mine-1' && log.puts[0].snippet.textOriginal === '直しました',
  )

  // back -> same position, card gone, no reload
  await page.goBack()
  await page.waitForSelector('dialog.watch', { state: 'detached' })
  const after = await nextTop()
  const scrollTop = await page.evaluate(() => document.querySelector('main').scrollTop)
  check(
    'back returns to the same place in the list',
    scrollTop > 0 && after >= 0 && after < 800 && Math.abs(after - before) < 60,
    `neighbour top ${before} -> ${after}, scrollTop ${scrollTop}`,
  )
  check(
    'watched card left the list',
    (await page.locator('.video-card').count()) === IDS.length - 1,
  )
  check('list was not reloaded', log.feed === feedLoads, `${feedLoads} -> ${log.feed}`)
  check('url is clean again', !page.url().includes('watch='))

  // next video, comment goes to the new one
  await page.locator('.video-card').nth(1).locator('.title').tap()
  await page.waitForSelector('dialog.watch[open]')
  const second = new URL(page.url()).searchParams.get('watch')
  check('a second video opens after the first', second === IDS[2], second)
  check(
    'portrait video fits the screen',
    await page.evaluate(() => {
      const r = document.querySelector('dialog.watch .player').getBoundingClientRect()
      return r.bottom <= window.innerHeight && r.width <= window.innerWidth && r.height > r.width
    }),
  )
  await page.frameLocator('dialog.watch iframe').locator('body').waitFor()
  await page.getByRole('textbox', { name: 'コメント', exact: true }).fill('二本目')
  await page.getByRole('button', { name: '送信' }).tap()
  await page.waitForSelector('.comment.own')
  check(
    'second comment is addressed to the second video',
    log.posts[1].body.snippet.videoId === IDS[2],
    log.posts[1].body.snippet.videoId,
  )
  check(
    'token is reused within the visit',
    (await page.evaluate(() => window.__gis.scopes.length)) === 1,
  )

  // delete: two steps
  await page.getByRole('button', { name: '削除', exact: true }).tap()
  check(
    'delete asks before deleting',
    log.deletes.length === 0 &&
      (await page.getByRole('button', { name: '削除確認' }).count()) === 1,
  )
  await page.getByRole('button', { name: '削除確認' }).tap()
  await page.waitForSelector('.comment.own', { state: 'detached' })
  check('delete removes that comment', log.deletes.join() === 'mine-2')

  // close button
  await page.getByRole('button', { name: '閉じる' }).tap()
  await page.waitForSelector('dialog.watch', { state: 'detached' })
  check(
    'close button returns to the list',
    !page.url().includes('watch=') &&
      (await page.locator('.video-card').count()) === IDS.length - 2,
  )

  await context.close()
})

test('failed sends keep the text, drafts survive closing, refused videos point to YouTube', async ({
  browser,
}) => {
  const { context, page, log } = await setup(browser, { gis: 'cancel' })
  await page.goto(BASE + '/')
  await page.locator('.video-card').first().locator('.thumbnail-link').tap()
  await page.waitForSelector('dialog.watch[open]')
  check(
    'long title stays on one line',
    await page.evaluate(() => {
      const t = document.querySelector('dialog.watch .title')
      return t.getBoundingClientRect().height < 30 && t.scrollWidth > t.clientWidth
    }),
  )
  const box = page.getByRole('textbox', { name: 'コメント', exact: true })
  await box.fill('残る本文')
  await page.getByRole('button', { name: '送信' }).tap()
  await page.waitForSelector('.compose .error')
  check(
    'cancelled authorization is explained',
    (await page.locator('.compose .error').innerText()) === 'Google の認可がキャンセルされました',
  )
  check(
    'text survives a cancelled authorization',
    (await box.inputValue()) === '残る本文' && log.posts.length === 0,
  )
  await page.evaluate(() => (window.__gis.mode = 'ok'))
  log.state.post = 'quota'
  await page.getByRole('button', { name: '送信' }).tap()
  await page.waitForFunction(() =>
    document.querySelector('.compose .error')?.innerText.includes('割り当て'),
  )
  check('quota exhaustion is explained and text survives', (await box.inputValue()) === '残る本文')
  log.state.post = 'unauthorized'
  await page.getByRole('button', { name: '送信' }).tap()
  await page.waitForFunction(() =>
    document.querySelector('.compose .error')?.innerText.includes('認可が切れました'),
  )
  log.state.post = 'ok'
  await page.getByRole('button', { name: '送信' }).tap()
  await page.waitForSelector('.comment.own')
  check(
    'a rejected token is requested again on retry',
    (await page.evaluate(() => window.__gis.scopes.length)) === 3,
    String(await page.evaluate(() => window.__gis.scopes.length)),
  )

  // draft survives closing by accident
  await box.fill('書きかけ')
  await page.goBack()
  await page.waitForSelector('dialog.watch', { state: 'detached' })
  await page.goForward()
  await page.waitForSelector('dialog.watch[open]')
  check(
    'draft and posted comment survive closing and reopening',
    (await page.getByRole('textbox', { name: 'コメント', exact: true }).inputValue()) ===
      '書きかけ' && (await page.locator('.comment.own').count()) === 1,
  )

  // embed blocked
  await page.waitForFunction(() => window.__players?.length >= 1)
  await page.evaluate(() => window.__players.at(-1).opts.events.onError({ data: 150 }))
  await page.waitForSelector('.player-error')
  const notice = page.locator('.player-error')
  check(
    'a video the player refuses shows a notice with a YouTube link',
    (await notice.innerText()).includes('この動画はここでは再生できません') &&
      (await notice.locator('a').getAttribute('href')) ===
        `https://www.youtube.com/watch?v=${IDS[0]}`,
  )

  // reload keeps the video open; title comes from the player
  await page.reload()
  await page.waitForSelector('dialog.watch[open]')
  await page.waitForFunction(
    () => document.querySelector('dialog.watch .title').innerText === 'プレイヤーが知らせた題',
  )
  check('reload reopens the same video', new URL(page.url()).searchParams.get('watch') === IDS[0])
  await page.keyboard.press('Escape')
  await page.waitForSelector('dialog.watch', { state: 'detached' })
  check(
    'closing after a reload drops the parameter and shows the list',
    !page.url().includes('watch=') && (await page.locator('.video-card').count()) > 0,
  )
  await context.close()
})

for (const [state, expect] of [
  ['disabled', 'この動画はコメントを受け付けていません'],
  ['quota', '割り当て切れ'],
  ['error', 'コメントを読み込めませんでした'],
  ['unavailable', null],
]) {
  test(`comments: ${state}`, async ({ browser }) => {
    const { context, page, log } = await setup(browser, { comments: state })
    await page.goto(BASE + '/?watch=' + IDS[3])
    await page.waitForSelector('dialog.watch[open]')
    await page.waitForFunction(
      () => !document.querySelector('.comments .spinner, .comments [class*=spinner]'),
    )
    await page.waitForTimeout(200)
    const text = await page.locator('.comments').innerText()
    const form = await page.locator('.compose').count()
    if (state === 'disabled')
      check('disabled comments: note, no form', text.trim() === expect && form === 0, text)
    if (state === 'quota')
      check('quota on reading: note, form still there', text.includes(expect) && form === 1, text)
    if (state === 'unavailable')
      check(
        'no API key: form only, nothing else',
        form === 1 &&
          (await page.locator('.comments .note, .comments .error, .comment').count()) === 0,
        text,
      )
    if (state === 'error') {
      check('read failure: reason and retry', text.includes(expect) && form === 1)
      log.state.comments = 'ok'
      await page.getByRole('button', { name: '再試行' }).tap()
      await page.waitForSelector('.comment')
      check('retry loads the comments', true)
    }
    await context.close()
  })
}

test('without a GIS client id comments can be read but not sent', async ({ browser }) => {
  const { context, page } = await setup(browser, { gisClientId: '' })
  await page.goto(BASE + '/?watch=' + IDS[3])
  await page.waitForSelector('.comment')
  check(
    'no GIS client id: reading works, sending is explained',
    (await page.locator('.compose').count()) === 0 &&
      (await page.locator('.comments .note').innerText()).includes('GIS_CLIENT_ID'),
  )
  await context.close()
})

test('history opens the watch view and its channel link leaves it', async ({ browser }) => {
  const { context, page } = await setup(browser)
  await page.goto(BASE + '/history')
  await page.waitForSelector('.video-card')
  await page.locator('.video-card').first().locator('.thumbnail-link').tap()
  await page.waitForSelector('dialog.watch[open]')
  check(
    'history opens the watch view too',
    context.pages().length === 1 &&
      new URL(page.url()).pathname === '/history' &&
      new URL(page.url()).searchParams.get('watch') === IDS[0],
  )
  // channel link leaves the watch view
  await page.locator('dialog.watch .channel').tap()
  await page.waitForSelector('dialog.watch', { state: 'detached' })
  check(
    'channel link in the watch view goes to the channel page',
    new URL(page.url()).pathname.startsWith('/channel/'),
  )
  await context.close()
})

for (const scheme of ['dark', 'light']) {
  test(`desktop keyboard and layout (${scheme})`, async ({ browser }) => {
    const { context, page, log } = await setup(browser, { width: 1280, height: 800, scheme })
    await page.goto(BASE + '/')
    await page.waitForSelector('.video-card')
    if (scheme === 'dark') {
      const [popup] = await Promise.all([
        context.waitForEvent('page'),
        page
          .locator('.video-card')
          .nth(3)
          .locator('.title')
          .click({ modifiers: ['Control'] }),
      ])
      check('ctrl+click still opens a new tab', context.pages().length === 2)
      await popup.close()
      check(
        'ctrl+click does not open the watch view',
        (await page.locator('dialog.watch').count()) === 0,
      )
    }
    await page.locator('.video-card').first().locator('.title').focus()
    await page.keyboard.press('Enter')
    await page.waitForSelector('dialog.watch[open]')
    await page.waitForSelector('.comment')
    const m = await sideBySide(page)
    check(`[${scheme}] desktop: video and comment box share the screen`, m.ok, JSON.stringify(m))
    check(
      `[${scheme}] desktop: the video keeps most of the width`,
      m.player.width >= m.iw * 0.7,
      JSON.stringify(m),
    )
    const focus = await page.evaluate(() => document.activeElement?.getAttribute('aria-label'))
    check(`[${scheme}] keyboard: focus lands in the dialog`, focus === '閉じる', focus)
    check(`[${scheme}] no horizontal scroll at 1280`, await noHScroll(page))
    await page.getByRole('textbox', { name: 'コメント', exact: true }).fill('ショートカットで送信')
    await page.keyboard.press('Control+Enter')
    await page.waitForSelector('.comment.own')
    check(`[${scheme}] ctrl+enter sends`, log.posts.length === 1)
    await page.keyboard.press('Escape')
    await page.waitForSelector('dialog.watch', { state: 'detached' })
    check(`[${scheme}] escape closes`, !page.url().includes('watch='))
    await context.close()
  })
}
test('nothing overflows sideways at 320px', async ({ browser }) => {
  const { context, page } = await setup(browser, { width: 320, height: 640 })
  await page.goto(BASE + '/?watch=' + IDS[0])
  await page.waitForSelector('.comment')
  check('no horizontal scroll at 320px', await noHScroll(page))
  await context.close()
})

for (const [device, width, height, share] of [
  ['tablet', 1304, 660, 0.7],
  ['phone', 915, 412, 0.6],
  ['small phone', 740, 360, 0.5],
]) {
  test(`${device} on its side: video, way back and comment box share the screen`, async ({
    browser,
  }) => {
    const { context, page } = await setup(browser, { width, height })
    await page.goto(BASE + '/?watch=' + IDS[0])
    await page.waitForSelector('.comment')
    const m = await sideBySide(page)
    check('nothing needs scrolling to reach', m.ok, JSON.stringify(m))
    check(
      'the video keeps its share of the width',
      m.player.width >= m.iw * share,
      JSON.stringify(m),
    )
    check(
      'the page itself does not scroll',
      (await noHScroll(page)) &&
        (await page.evaluate(() => {
          const d = document.querySelector('dialog.watch')
          return d.scrollHeight <= d.clientHeight + 1
        })),
    )
    // The comments scroll in their own column while the video stays put.
    const before = m.player.top
    await page.evaluate(() => document.querySelector('dialog.watch .body').scrollTo(0, 500))
    const after = await page.evaluate(() => {
      const b = document.querySelector('dialog.watch .body')
      return {
        top: document.querySelector('dialog.watch .player').getBoundingClientRect().top,
        scrolled: b.scrollTop,
        overflows: b.scrollHeight > b.clientHeight + 1,
      }
    })
    check(
      'comments scroll beside a still video',
      after.top === before && (after.scrolled > 0 || !after.overflows),
      JSON.stringify(after),
    )
    await context.close()
  })
}

test('the list is left alone while a video plays', async ({ browser }) => {
  const { context, page, log } = await setup(browser)
  await page.clock.install()
  await page.goto(BASE + '/')
  await page.waitForSelector('.video-card')
  await page.clock.fastForward(61_000)
  await page.waitForTimeout(100)
  const browsing = log.feed
  check('list refreshes itself while browsing', browsing === 2, String(browsing))
  await page.locator('.video-card').first().locator('.thumbnail-link').tap()
  await page.waitForSelector('dialog.watch[open]')
  await page.clock.fastForward(10 * 60_000)
  await page.waitForTimeout(100)
  check(
    'list is not reloaded during ten minutes of watching',
    log.feed === browsing,
    `${browsing} -> ${log.feed}`,
  )
  await page.getByRole('button', { name: '閉じる' }).tap()
  await page.waitForSelector('dialog.watch', { state: 'detached' })
  await page.clock.fastForward(61_000)
  await page.waitForTimeout(100)
  check(
    'list refreshes again after closing',
    log.feed === browsing + 1,
    `${browsing} -> ${log.feed}`,
  )
  await context.close()
})
