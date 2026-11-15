import { describe, test, expect } from 'vitest'
import { WriteError, sendAsViewer, writeErrorMessage } from './youtube-write.js'

describe('writeErrorMessage', () => {
  test("prefers the caller's wording for a reason it knows", () => {
    const error = new WriteError(403, 'quotaExceeded')
    expect(writeErrorMessage(error, { quotaExceeded: '独自の文' })).toBe('独自の文')
    expect(writeErrorMessage(error)).toMatch(/割り当て/)
  })
})

describe('sendAsViewer', () => {
  const request = { url: 'https://example.test/x', init: { method: 'POST', body: '{}' } }

  function tokens(values) {
    const calls = { get: 0, invalidate: 0 }
    return {
      calls,
      get: async () => values[calls.get++],
      invalidate: () => calls.invalidate++,
    }
  }

  test('sends with the bearer token and returns the parsed body', async () => {
    const seen = []
    const fetchFn = async (url, init) => {
      seen.push({ url, init })
      return { ok: true, status: 200, json: async () => ({ id: 'c1' }) }
    }
    const body = await sendAsViewer(request, { tokens: tokens(['T1']), fetchFn })
    expect(body).toEqual({ id: 'c1' })
    expect(seen[0].url).toBe('https://example.test/x')
    expect(seen[0].init.headers.Authorization).toBe('Bearer T1')
    expect(seen[0].init.headers['Content-Type']).toBe('application/json')
    expect(seen[0].init.body).toBe('{}')
  })

  test('declares a JSON body only when there is one', async () => {
    const seen = []
    const fetchFn = async (url, init) => {
      seen.push(init)
      return { ok: true, status: 204, json: async () => Promise.reject() }
    }
    const bare = { url: 'https://example.test/x', init: { method: 'POST' } }
    await sendAsViewer(bare, { tokens: tokens(['T1']), fetchFn })
    expect(seen[0].headers).toEqual({ Authorization: 'Bearer T1' })
  })

  test('returns null for an empty success body (delete)', async () => {
    const fetchFn = async () => ({ ok: true, status: 204, json: async () => Promise.reject() })
    expect(await sendAsViewer(request, { tokens: tokens(['T1']), fetchFn })).toBeNull()
  })

  test('throws the API reason and drops a rejected token', async () => {
    const store = tokens(['T1'])
    const fetchFn = async () => ({
      ok: false,
      status: 401,
      json: async () => ({ error: { errors: [{ reason: 'authError' }] } }),
    })
    await expect(sendAsViewer(request, { tokens: store, fetchFn })).rejects.toMatchObject({
      status: 401,
      reason: 'authError',
    })
    expect(store.calls.invalidate).toBe(1)
  })

  test('keeps the token on failures that are not about authorization', async () => {
    const store = tokens(['T1'])
    const fetchFn = async () => ({
      ok: false,
      status: 403,
      json: async () => ({ error: { errors: [{ reason: 'commentsDisabled' }] } }),
    })
    await expect(sendAsViewer(request, { tokens: store, fetchFn })).rejects.toMatchObject({
      status: 403,
      reason: 'commentsDisabled',
    })
    expect(store.calls.invalidate).toBe(0)
  })
})
