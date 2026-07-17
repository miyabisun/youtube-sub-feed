/**
 * Maps an Error thrown during channel sync to a user-facing Japanese message.
 * @param {Error} error
 * @returns {string}
 */
export function toUserMessage(error) {
  const msg = error?.message ?? ''

  // GIS SDK not loaded — preserve the existing message as-is
  if (msg.startsWith('GIS SDK がロード')) {
    return msg
  }

  // User cancelled or closed the Google OAuth popup
  if (msg.includes('cancelled') || msg.includes('popup_closed')) {
    return 'Google 認可がキャンセルされました'
  }

  // YouTube Data API responded with an error (fetchAllSubscriptions throws "YouTube API error: ...")
  if (msg.startsWith('YouTube API error:')) {
    return 'YouTube API への接続に失敗しました（時間を置いて再試行してください）'
  }

  // Server-side sync endpoint error (fetcher throws "<status> <statusText>")
  if (msg === 'Unauthorized' || /^\d{3} /.test(msg)) {
    return 'サーバー側でのチャンネル同期に失敗しました'
  }

  // User dismissed the 0-channel confirmation dialog (existing behaviour)
  if (msg === 'チャンネル同期をキャンセルしました') {
    return msg
  }

  // Unexpected error (implementation bug, network issue, etc.)
  return '予期しないエラーが発生しました。コンソールを確認してください。'
}
