import { describe, test, expect, beforeAll } from 'vitest'

// router.svelte.js runs top-level code that touches `window`/`history` on import
// (it registers a popstate listener and syncs the initial route). Stub those
// globals before importing so `matchRoute` can be exercised in a non-DOM env.
let matchRoute

beforeAll(async () => {
  globalThis.window = {
    __BASE_PATH__: '',
    location: { pathname: '/' },
    addEventListener: () => {},
  }
  globalThis.history = { pushState: () => {} }
  ;({ matchRoute } = await import('./router.svelte.js'))
})

describe('matchRoute', () => {
  test('matches the root path to index 0', () => {
    expect(matchRoute('/')).toEqual({ index: 0, params: {} })
  })

  test('matches /channels to its route index', () => {
    expect(matchRoute('/channels')).toEqual({ index: 2, params: {} })
  })

  test('matches /history to its route index', () => {
    expect(matchRoute('/history')).toEqual({ index: 6, params: {} })
  })

  test('captures the id param for /group/:id', () => {
    expect(matchRoute('/group/abc')).toEqual({ index: 1, params: { id: 'abc' } })
  })

  test('captures the id param for /channel/:id', () => {
    expect(matchRoute('/channel/UC123')).toEqual({ index: 3, params: { id: 'UC123' } })
  })

  test('decodes percent-encoded params via decodeURIComponent', () => {
    // %E3%81%82 is the UTF-8 encoding of "あ".
    expect(matchRoute('/group/%E3%81%82')).toEqual({ index: 1, params: { id: 'あ' } })
  })

  test('falls back to index 0 with empty params when nothing matches', () => {
    expect(matchRoute('/does/not/exist')).toEqual({ index: 0, params: {} })
  })
})
