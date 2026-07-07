import { describe, test, expect, vi, beforeEach } from 'vitest'

// Mock the router so importing fetcher does not pull in window-dependent code,
// and so we can assert navigate('/login') is invoked on 401.
const { navigate } = vi.hoisted(() => ({ navigate: vi.fn() }))
vi.mock('$lib/router.svelte.js', () => ({ navigate }))

import fetcher from './fetcher.js'

describe('fetcher', () => {
  beforeEach(() => {
    navigate.mockReset()
    globalThis.fetch = vi.fn()
  })

  test('on 401 redirects to /login and throws Unauthorized', async () => {
    globalThis.fetch.mockResolvedValue({
      status: 401,
      ok: false,
      statusText: 'Unauthorized',
      json: async () => ({}),
    })

    await expect(fetcher('/api/x')).rejects.toThrow('Unauthorized')
    expect(navigate).toHaveBeenCalledWith('/login')
  })

  test('on a non-ok response throws "<status> <statusText>" without redirecting', async () => {
    globalThis.fetch.mockResolvedValue({
      status: 500,
      ok: false,
      statusText: 'Internal Server Error',
      json: async () => ({}),
    })

    await expect(fetcher('/api/x')).rejects.toThrow('500 Internal Server Error')
    expect(navigate).not.toHaveBeenCalled()
  })

  test('on success resolves with the parsed JSON body', async () => {
    const data = { hello: 'world' }
    globalThis.fetch.mockResolvedValue({
      status: 200,
      ok: true,
      statusText: 'OK',
      json: async () => data,
    })

    await expect(fetcher('/api/x')).resolves.toEqual(data)
    expect(navigate).not.toHaveBeenCalled()
  })

  test('forwards url and options to fetch', async () => {
    globalThis.fetch.mockResolvedValue({
      status: 200,
      ok: true,
      statusText: 'OK',
      json: async () => ({}),
    })
    const options = { method: 'POST', body: '{}' }

    await fetcher('/api/y', options)

    expect(globalThis.fetch).toHaveBeenCalledWith('/api/y', options)
  })
})
