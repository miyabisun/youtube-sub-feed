import { describe, test, expect } from 'vitest'
import { watchIdFromSearch, searchWithWatch } from './watch-url.js'

describe('watchIdFromSearch', () => {
  test('reads the video id from the watch parameter', () => {
    expect(watchIdFromSearch('?watch=stw7lYY3W3I')).toBe('stw7lYY3W3I')
  })

  test('finds it among other parameters', () => {
    expect(watchIdFromSearch('?a=1&watch=a-b_c-d_e-f&b=2')).toBe('a-b_c-d_e-f')
  })

  test('is null when no video is open', () => {
    expect(watchIdFromSearch('')).toBeNull()
    expect(watchIdFromSearch('?a=1')).toBeNull()
  })

  test('rejects values that are not a video id', () => {
    expect(watchIdFromSearch('?watch=short')).toBeNull()
    expect(watchIdFromSearch('?watch=stw7lYY3W3I%22%3E')).toBeNull()
  })
})

describe('searchWithWatch', () => {
  test('adds the watch parameter', () => {
    expect(searchWithWatch('', 'stw7lYY3W3I')).toBe('?watch=stw7lYY3W3I')
  })

  test('keeps the other parameters', () => {
    expect(searchWithWatch('?a=1', 'stw7lYY3W3I')).toBe('?a=1&watch=stw7lYY3W3I')
  })

  test('replaces the video that was open', () => {
    expect(searchWithWatch('?watch=stw7lYY3W3I', 'a-b_c-d_e-f')).toBe('?watch=a-b_c-d_e-f')
  })

  test('removes the parameter when closing', () => {
    expect(searchWithWatch('?a=1&watch=stw7lYY3W3I', null)).toBe('?a=1')
    expect(searchWithWatch('?watch=stw7lYY3W3I', null)).toBe('')
  })
})
