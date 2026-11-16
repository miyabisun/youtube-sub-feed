import { describe, expect, test } from 'vitest'
import { isStale } from './app-version.js'

describe('isStale', () => {
  test('the page is current while the server runs the release it came from', () => {
    expect(isStale('0.4.4', '0.4.4')).toBe(false)
  })

  test('the page is stale once the server runs another release', () => {
    expect(isStale('0.4.4', '0.4.5')).toBe(true)
    // A rollback serves another client as well.
    expect(isStale('0.4.5', '0.4.4')).toBe(true)
  })

  test('a page that carries no version is never stale, whatever the server says', () => {
    // The dev server hands the page out without a version; reloading would not give it one.
    expect(isStale('', '0.4.5')).toBe(false)
    expect(isStale(undefined, '0.4.5')).toBe(false)
  })

  test('a server version that could not be read is no reason to reload', () => {
    expect(isStale('0.4.4', null)).toBe(false)
    expect(isStale('0.4.4', undefined)).toBe(false)
    expect(isStale('0.4.4', '')).toBe(false)
  })

  test('an answer that is not a version string is no reason to reload', () => {
    expect(isStale('0.4.4', 404)).toBe(false)
    expect(isStale('0.4.4', { version: '0.4.5' })).toBe(false)
    expect(isStale('0.4.4', true)).toBe(false)
  })
})
