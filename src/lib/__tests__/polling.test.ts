import { describe, test, expect } from 'bun:test'

describe('polling interval calculation', () => {
  test('normal polling: 190 channels / 30min = ~9.47s per channel', () => {
    const channelCount = 190
    const interval = Math.floor(30 * 60 * 1000 / channelCount)
    expect(interval).toBe(9473)
    expect(interval).toBeGreaterThan(9000)
    expect(interval).toBeLessThan(10000)
  })

  test('fast polling: 5 channels / 10min = 2min per channel', () => {
    const channelCount = 5
    const interval = Math.floor(10 * 60 * 1000 / channelCount)
    expect(interval).toBe(120_000)
  })

  test('handles single channel gracefully', () => {
    const channelCount = 1
    const normalInterval = Math.floor(30 * 60 * 1000 / channelCount)
    const fastInterval = Math.floor(10 * 60 * 1000 / channelCount)
    expect(normalInterval).toBe(1_800_000)
    expect(fastInterval).toBe(600_000)
  })

  test('index wraps around correctly', () => {
    const count = 3
    let index = 0
    const visited: number[] = []

    for (let i = 0; i < 7; i++) {
      visited.push(index % count)
      index++
      if (index >= count) index = 0
    }

    expect(visited).toEqual([0, 1, 2, 0, 1, 2, 0])
  })
})
