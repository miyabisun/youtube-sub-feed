import { describe, test, expect, vi, beforeEach } from 'vitest'

// Mock the fetcher so loadGroups can be tested without any network / router.
const { fetcher } = vi.hoisted(() => ({ fetcher: vi.fn() }))
vi.mock('$lib/fetcher.js', () => ({ default: fetcher }))
vi.mock('$lib/config.js', () => ({ default: { path: { api: '/api' } } }))

// Each test re-imports the module for a fresh `loaded`/`groups` module state.
async function freshModule() {
  vi.resetModules()
  return import('./groups.svelte.js')
}

describe('loadGroups', () => {
  beforeEach(() => {
    fetcher.mockReset()
  })

  test('fetches the group list on first load', async () => {
    fetcher.mockResolvedValue([{ id: 1, name: 'G1' }])
    const { loadGroups, getGroups } = await freshModule()

    await loadGroups()

    expect(fetcher).toHaveBeenCalledTimes(1)
    expect(fetcher).toHaveBeenCalledWith('/api/groups')
    expect(getGroups()).toEqual([{ id: 1, name: 'G1' }])
  })

  test('short-circuits when already loaded and not forced', async () => {
    fetcher.mockResolvedValue([{ id: 1, name: 'G1' }])
    const { loadGroups } = await freshModule()

    await loadGroups()
    await loadGroups() // cache hit: loaded && !force

    expect(fetcher).toHaveBeenCalledTimes(1)
  })

  test('re-fetches when force is true even after loading', async () => {
    fetcher.mockResolvedValue([{ id: 1, name: 'G1' }])
    const { loadGroups } = await freshModule()

    await loadGroups()
    await loadGroups(true) // force bypasses the cache

    expect(fetcher).toHaveBeenCalledTimes(2)
  })
})
