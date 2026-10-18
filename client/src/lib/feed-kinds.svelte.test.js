import { describe, test, expect, vi } from 'vitest'

// Each test re-imports the module for a fresh session state.
async function freshModule() {
  vi.resetModules()
  return import('./feed-kinds.svelte.js')
}

describe('feed kinds', () => {
  test('both kinds start on at launch', async () => {
    const { feedKinds, kindsParam } = await freshModule()
    expect(feedKinds).toEqual({ video: true, post: true })
    expect(kindsParam(feedKinds)).toBe('video,post')
  })

  test('the query lists only the kinds that are on', async () => {
    const { kindsParam } = await freshModule()
    expect(kindsParam({ video: false, post: true })).toBe('post')
    expect(kindsParam({ video: true, post: false })).toBe('video')
    expect(kindsParam({ video: false, post: false })).toBe('')
  })

  test('the toggles are shared module state, so remounting the feed keeps them', async () => {
    const first = await import('./feed-kinds.svelte.js')
    first.feedKinds.post = false
    const again = await import('./feed-kinds.svelte.js')
    expect(again.feedKinds.post).toBe(false)
  })
})
