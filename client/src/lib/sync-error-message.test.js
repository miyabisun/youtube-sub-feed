import { describe, test, expect } from 'vitest';
import { toUserMessage } from './sync-error-message.js';

describe('toUserMessage', () => {
  test('keeps the existing message when the GIS SDK has not loaded', () => {
    const msg = 'GIS SDK がロードされていません。ページをリロードしてください。';
    expect(toUserMessage(new Error(msg))).toBe(msg);
  });

  test('displays a friendly message when google auth is cancelled', () => {
    expect(toUserMessage(new Error('token request cancelled'))).toBe(
      'Google 認可がキャンセルされました',
    );
  });

  test('displays a friendly message when google auth popup is closed by user', () => {
    expect(toUserMessage(new Error('popup_closed_by_user'))).toBe(
      'Google 認可がキャンセルされました',
    );
  });

  test('surfaces a retry-friendly message when the youtube api fails', () => {
    expect(toUserMessage(new Error('YouTube API error: 503'))).toBe(
      'YouTube API への接続に失敗しました（時間を置いて再試行してください）',
    );
  });

  test('surfaces a generic server failure message when the sync endpoint errors', () => {
    expect(toUserMessage(new Error('500 Internal Server Error'))).toBe(
      'サーバー側でのチャンネル同期に失敗しました',
    );
  });

  test('surfaces a generic server failure message for 401 Unauthorized', () => {
    expect(toUserMessage(new Error('Unauthorized'))).toBe(
      'サーバー側でのチャンネル同期に失敗しました',
    );
  });

  test('falls back to a generic message for unexpected errors', () => {
    expect(toUserMessage(new TypeError('foo is not a function'))).toBe(
      '予期しないエラーが発生しました。コンソールを確認してください。',
    );
  });

  test('preserves the cancel message when the user dismisses the 0-channel dialog', () => {
    expect(toUserMessage(new Error('チャンネル同期をキャンセルしました'))).toBe(
      'チャンネル同期をキャンセルしました',
    );
  });

  // Defensive: a null/undefined argument must not throw (error?.message ?? '')
  // and should fall through to the generic message.
  test('falls back to the generic message when error is null', () => {
    expect(toUserMessage(null)).toBe(
      '予期しないエラーが発生しました。コンソールを確認してください。',
    );
  });

  test('falls back to the generic message when error is undefined', () => {
    expect(toUserMessage(undefined)).toBe(
      '予期しないエラーが発生しました。コンソールを確認してください。',
    );
  });
});
