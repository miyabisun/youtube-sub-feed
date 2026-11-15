// How focus is drawn: a text field recolours its own border and nothing else,
// while controls without a border of their own keep the shared ring.
import { test } from '@playwright/test'
import { BASE, IDS, check, setup } from './harness.js'

const GROUPS = [{ id: 1, name: 'ゲーム', sort_order: 0 }]
const CHANNELS = [{ id: 'UC' + 'x'.repeat(22), title: 'チャンネル', group_names: null }]

// What is drawn around the focused element.
const focused = (page) =>
  page.evaluate(() => {
    const el = document.activeElement
    const style = getComputedStyle(el)
    const probe = document.body.appendChild(document.createElement('i'))
    probe.style.color = 'var(--c-accent)'
    const accent = getComputedStyle(probe).color
    probe.remove()
    return {
      el: el.tagName.toLowerCase() + (el.tagName === 'INPUT' ? `[${el.type}]` : ''),
      ring: style.outlineStyle === 'none' ? 0 : parseFloat(style.outlineWidth),
      accentBorder: style.borderTopColor === accent && parseFloat(style.borderTopWidth) > 0,
    }
  })

async function oneLine(page, name, field) {
  await field.click()
  const f = await focused(page)
  check(`${name}: one accent line, no ring`, f.ring === 0 && f.accentBorder, JSON.stringify(f))
}

for (const scheme of ['dark', 'light']) {
  test(`a focused text field shows one line (${scheme})`, async ({ browser }) => {
    const options = { scheme, width: 1280, height: 800, groups: GROUPS, channels: CHANNELS }
    const { context, page } = await setup(browser, options)

    await page.goto(BASE + '/?watch=' + IDS[0])
    const box = page.getByRole('textbox', { name: 'コメント', exact: true })
    await oneLine(page, 'comment box', box)
    await box.fill('直す前')
    await page.getByRole('button', { name: '送信' }).click()
    await page.getByRole('button', { name: '編集' }).click()
    await oneLine(page, 'comment edit box', page.getByRole('textbox', { name: 'コメントを編集' }))

    await page.goto(BASE + '/channels')
    await oneLine(page, 'channel id', page.getByPlaceholder('チャンネルID'))
    await oneLine(page, 'channel search', page.getByPlaceholder('チャンネル名で検索'))

    await page.goto(BASE + '/settings')
    await oneLine(page, 'new group name', page.getByPlaceholder('新しいグループ名'))
    await page.locator('.group-name').click()
    await oneLine(page, 'group rename', page.locator('.edit-input'))

    // Reached by keyboard, a button and a checkbox keep the ring.
    await page.getByRole('button', { name: '割当' }).click()
    await page.waitForSelector('.assign-row input[type=checkbox]')
    await page.getByPlaceholder('新しいグループ名').focus()
    const rings = {}
    for (let i = 0; i < 20 && !(rings['button'] && rings['input[checkbox]']); i++) {
      await page.keyboard.press('Tab')
      const f = await focused(page)
      rings[f.el] ??= f.ring
    }
    check(
      'a button reached by keyboard keeps the ring',
      rings['button'] === 2,
      JSON.stringify(rings),
    )
    check(
      'a checkbox reached by keyboard keeps the ring',
      rings['input[checkbox]'] === 2,
      JSON.stringify(rings),
    )
    await context.close()

    // The group select replaces the tabs on a phone.
    const phone = await setup(browser, { scheme, groups: GROUPS })
    await phone.page.goto(BASE + '/')
    await phone.page.waitForSelector('.video-card')
    await phone.page.locator('select.group-select').focus()
    const f = await focused(phone.page)
    check(
      'group select: one accent line, no ring',
      f.ring === 0 && f.accentBorder,
      JSON.stringify(f),
    )
    await phone.context.close()
  })
}
