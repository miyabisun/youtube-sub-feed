import { Client, GatewayIntentBits, EmbedBuilder, type TextChannel } from 'discord.js'

let client: Client | null = null
let channelId: string | null = null

export async function initDiscordClient(): Promise<void> {
  const token = process.env.DISCORD_TOKEN
  channelId = process.env.DISCORD_CHANNEL_ID || null

  if (!token || !channelId) {
    console.log('[discord] Token or channel ID not set, notifications disabled')
    return
  }

  client = new Client({ intents: [GatewayIntentBits.Guilds] })

  try {
    await client.login(token)
    console.log('[discord] Bot connected')
  } catch (e) {
    console.error('[discord] Failed to connect:', e)
    client = null
  }
}

export async function notifyNewVideo(video: {
  id: string
  title: string
  channel_title: string
  thumbnail_url: string | null
  published_at: string | null
  is_short: number
}): Promise<void> {
  if (!client || !channelId) return

  try {
    const channel = await client.channels.fetch(channelId) as TextChannel
    if (!channel) return

    const url = video.is_short
      ? `https://www.youtube.com/shorts/${video.id}`
      : `https://www.youtube.com/watch?v=${video.id}`

    const embed = new EmbedBuilder()
      .setAuthor({ name: video.channel_title })
      .setTitle(video.title)
      .setURL(url)
      .setColor(0xd93025)

    if (video.thumbnail_url) embed.setImage(video.thumbnail_url)
    if (video.published_at) embed.setTimestamp(new Date(video.published_at))

    await channel.send({ embeds: [embed] })
  } catch (e) {
    console.error('[discord] Failed to send notification:', e)
  }
}

export async function notifySetupComplete(channelCount: number, videoCount: number): Promise<void> {
  if (!client || !channelId) return

  try {
    const channel = await client.channels.fetch(channelId) as TextChannel
    if (!channel) return

    const embed = new EmbedBuilder()
      .setTitle('初回セットアップ完了')
      .setDescription(`${channelCount}チャンネル、${videoCount}件の動画を取得しました`)
      .setColor(0x00c853)
      .setTimestamp()

    await channel.send({ embeds: [embed] })
  } catch (e) {
    console.error('[discord] Failed to send setup notification:', e)
  }
}
