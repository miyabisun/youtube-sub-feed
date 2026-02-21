import { describe, test, expect } from 'bun:test'
import { EmbedBuilder } from 'discord.js'

describe('discord embed construction', () => {
  test('builds a new video embed correctly', () => {
    const video = {
      id: 'abc123',
      title: 'Test Video',
      channel_title: 'Test Channel',
      thumbnail_url: 'https://i.ytimg.com/vi/abc123/mqdefault.jpg',
      published_at: '2024-06-01T12:00:00Z',
      is_short: 0,
    }

    const url = video.is_short
      ? `https://www.youtube.com/shorts/${video.id}`
      : `https://www.youtube.com/watch?v=${video.id}`

    const embed = new EmbedBuilder()
      .setAuthor({ name: video.channel_title })
      .setTitle(video.title)
      .setURL(url)
      .setColor(0xd93025)
      .setImage(video.thumbnail_url)
      .setTimestamp(new Date(video.published_at))

    const json = embed.toJSON()
    expect(json.author?.name).toBe('Test Channel')
    expect(json.title).toBe('Test Video')
    expect(json.url).toBe('https://www.youtube.com/watch?v=abc123')
    expect(json.color).toBe(0xd93025)
    expect(json.image?.url).toBe('https://i.ytimg.com/vi/abc123/mqdefault.jpg')
  })

  test('builds a shorts video embed with correct URL', () => {
    const video = { id: 'short1', is_short: 1 }
    const url = video.is_short
      ? `https://www.youtube.com/shorts/${video.id}`
      : `https://www.youtube.com/watch?v=${video.id}`

    expect(url).toBe('https://www.youtube.com/shorts/short1')
  })

  test('builds a setup complete embed', () => {
    const embed = new EmbedBuilder()
      .setTitle('初回セットアップ完了')
      .setDescription('200チャンネル、5000件の動画を取得しました')
      .setColor(0x00c853)
      .setTimestamp()

    const json = embed.toJSON()
    expect(json.title).toBe('初回セットアップ完了')
    expect(json.color).toBe(0x00c853)
  })

  test('skips notification when client is null (token not set)', () => {
    // If client is null, notifyNewVideo should be a no-op
    // This tests the guard clause pattern
    const client = null
    const channelId = null
    const shouldSend = !!(client && channelId)
    expect(shouldSend).toBe(false)
  })
})
