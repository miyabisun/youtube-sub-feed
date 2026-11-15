import { describe, test, expect } from 'vitest'
import { createTokenCache } from './gis.js'

function setup(replies) {
  let now = 1_000_000
  let requests = 0
  const cache = createTokenCache(
    async () => replies[requests++],
    () => now,
  )
  return {
    cache,
    advance: (ms) => (now += ms),
    requests: () => requests,
  }
}

describe('createTokenCache', () => {
  test('asks once and reuses the token while it is valid', async () => {
    const t = setup([{ accessToken: 'T1', expiresIn: 3600 }])
    expect(await t.cache.get()).toBe('T1')
    t.advance(30 * 60_000)
    expect(await t.cache.get()).toBe('T1')
    expect(t.requests()).toBe(1)
  })

  test('asks again shortly before the token expires', async () => {
    const t = setup([
      { accessToken: 'T1', expiresIn: 3600 },
      { accessToken: 'T2', expiresIn: 3600 },
    ])
    await t.cache.get()
    t.advance(3600_000 - 30_000)
    expect(await t.cache.get()).toBe('T2')
  })

  test('asks again after the token was invalidated', async () => {
    const t = setup([
      { accessToken: 'T1', expiresIn: 3600 },
      { accessToken: 'T2', expiresIn: 3600 },
    ])
    await t.cache.get()
    t.cache.invalidate()
    expect(await t.cache.get()).toBe('T2')
  })

  test('does not cache a failed request', async () => {
    let attempts = 0
    const cache = createTokenCache(
      async () => {
        if (attempts++ === 0) throw new Error('popup_closed')
        return { accessToken: 'T1', expiresIn: 3600 }
      },
      () => 0,
    )
    await expect(cache.get()).rejects.toThrow('popup_closed')
    expect(await cache.get()).toBe('T1')
  })
})
