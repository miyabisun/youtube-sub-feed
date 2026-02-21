# novel-server 設計リファレンス

youtube-sub-feed が準拠する、novel-server（同一マシン上の別プロジェクト）の設計方針・規約の抜粋。
実装時に novel-server のソースコードを直接読む許可あり。

## プロジェクト構成

```
novel-server/
├── src/                     # バックエンド (TypeScript)
│   ├── index.ts             # エントリポイント & Hono アプリ設定
│   ├── db/
│   │   ├── index.ts         # Drizzle + SQLite 初期化
│   │   └── schema.ts        # テーブルスキーマ定義
│   ├── lib/
│   │   ├── cache.ts         # インメモリキャッシュ (TTL ベース)
│   │   ├── favorite-sync.ts # バックグラウンド定期同期
│   │   ├── init.ts          # DB スキーマ自動初期化
│   │   └── spa.ts           # SPA インデックス配信 & BASE_PATH 対応
│   ├── modules/             # ビジネスロジック (サイト別モジュール)
│   └── routes/              # API ルートハンドラ
├── client/                  # フロントエンド (Svelte 5)
│   ├── src/
│   │   ├── App.svelte       # ルートコンポーネント
│   │   ├── main.js          # エントリポイント
│   │   ├── global.sass      # CSS トークン & グローバルスタイル
│   │   ├── lib/
│   │   │   ├── config.js    # API パス設定
│   │   │   ├── fetcher.js   # fetch ラッパー
│   │   │   ├── router.svelte.js  # SPA ルーター (Svelte 5 $state)
│   │   │   └── components/  # 共通コンポーネント
│   │   └── pages/           # ページコンポーネント
│   └── vite.config.js
├── docs/                    # ドキュメント
├── CLAUDE.md                # プロジェクト固有ルール
├── package.json
├── tsconfig.json
├── drizzle.config.ts
└── Dockerfile
```

## 技術スタック

| 項目 | 技術 | バージョン |
|------|------|-----------|
| ランタイム | Bun | latest |
| フレームワーク | Hono | ^4.7.2 |
| ORM | Drizzle ORM | ^0.39.0 |
| DB | SQLite (bun:sqlite) | - |
| フロントエンド | Svelte 5 | ^5.0.0 |
| ビルドツール | Vite | ^6.0.0 |
| スタイリング | Sass | ^1.60.0 |
| TypeScript | strict: true | ^5.7.3 |

## package.json スクリプト

```json
{
  "type": "module",
  "scripts": {
    "dev": "cd client && bunx vite build --watch & bun --env-file=.env src/index.ts",
    "setup": "bun install && cd client && bun install && test -f .env || cp .env.example .env",
    "build:client": "cd client && bun install && bun run build",
    "build": "bun run build:client",
    "start": "bun --env-file=.env src/index.ts"
  }
}
```

## tsconfig.json

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "bundler",
    "strict": true,
    "skipLibCheck": true,
    "outDir": "dist",
    "rootDir": "src",
    "resolveJsonModule": true,
    "declaration": true,
    "sourceMap": true,
    "types": ["bun-types"]
  },
  "include": ["src"],
  "exclude": ["node_modules", "dist", "client"]
}
```

## drizzle.config.ts

```typescript
{
  schema: './src/db/schema.ts',
  dialect: 'sqlite',
  dbCredentials: {
    url: process.env.DATABASE_PATH || './novel.db'
  }
}
```

## DB 初期化パターン (src/db/index.ts)

```typescript
import { drizzle } from 'drizzle-orm/bun-sqlite'
import { Database } from 'bun:sqlite'

const sqlite = new Database(process.env.DATABASE_PATH || './novel.db')
// WAL モード & パフォーマンス PRAGMA
sqlite.exec('PRAGMA journal_mode = WAL')
sqlite.exec('PRAGMA synchronous = NORMAL')
sqlite.exec('PRAGMA cache_size = -64000')
sqlite.exec('PRAGMA temp_store = MEMORY')

export const db = drizzle(sqlite)
```

## インメモリキャッシュ (src/lib/cache.ts)

- Map ベースの TTL キャッシュ
- 最大 10,000 エントリ
- 1時間ごとにスイープ
- FIFO で最古エントリを削除

```typescript
// 使用例
cache.get(key)          // TTL 内ならヒット
cache.set(key, value, ttl)
cache.delete(key)       // 手動破棄
```

## バックグラウンド同期パターン (src/lib/favorite-sync.ts)

2種類の同期パターンを使用:

### 一括同期 (なろう・ノクターン)
```typescript
startSyosetuSync('narou', 10 * 60 * 1000)  // 10分間隔
// → fetchData() で全件一括取得 → トランザクションで一括更新
```

### ラウンドロビン同期 (カクヨム)
```typescript
startKakuyomuSync()  // 1時間で全件を循環
// → 1件ずつ fetchDatum() → 均等間隔でスケジューリング
// → interval = Math.floor(3_600_000 / count)
```

## SPA 配信 (src/lib/spa.ts)

- `client/build/` の静的ファイルを Hono の `serveStatic` で配信
- SPA のフォールバック: 未マッチのルートに `index.html` を返す
- `BASE_PATH` 対応: `<base>` タグと `window.__BASE_PATH__` を動的注入

## API ルートの規約

```typescript
// エラーレスポンス
c.json({ error: 'Invalid type' }, 400)
c.json({ error: 'Failed to fetch ranking' }, 502)

// リトライパターン (最大3回、指数バックオフ)
for (let i = 0; i < 3; i++) {
  try { /* ... */ } catch (e) {
    if (i < 2) await new Promise(r => setTimeout(r, 500 * (i + 1)))
  }
}
```

## フロントエンド規約

### ルーター (router.svelte.js)
- 正規表現ベースのパターンマッチング
- Svelte 5 の `$state` ルーンで状態管理
- `router.index` でページ切り替え、`router.params` でパラメータ取得

### ヘッダー (Header.svelte)
- タブ型ナビゲーション
- ページインデックスに応じてアクティブ表示

### 通信 (fetcher.js)
- fetch ラッパー（エラーハンドリング & JSON パース）
- `config.path.api` 経由で API パスを解決

### デザインシステム (global.sass)

```sass
// スペーシング (黄金比 φ ≈ 1.618)
--sp-1: 4px
--sp-2: 6px
--sp-3: 10px
--sp-4: 16px
--sp-5: 26px
--sp-6: 42px

// フォントサイズ
--fs-xs: 0.72rem
--fs-sm: 0.89rem
--fs: 1rem
--fs-md: 1.12rem
--fs-lg: 1.40rem
--fs-xl: 1.62rem

// ボーダーラジアス
--radius-sm: 4px
--radius: 6px
--radius-lg: 10px

// セマンティックカラー
--c-bg           // 背景
--c-surface      // カード面
--c-text         // 主テキスト
--c-text-soft    // 副テキスト
--c-text-faint   // 最淡テキスト
--c-accent-*     // アクセント
--c-danger-*     // 危険
--c-fav-*        // お気に入り
```

設計思想: LiftKit ベース + 黄金比スケーリング

## Dockerfile (マルチステージビルド)

```dockerfile
# Stage 1: Builder
FROM oven/bun:1
# サーバー依存 + クライアント依存をインストール
# フロントエンドビルド → /dist/ に集約

# Stage 2: Production
FROM oven/bun:1-slim
# /dist をコピー、本番依存のみ
# NODE_ENV=production, PORT=3000
# CMD: ["bun", "run", "src/index.ts"]
```

## CLAUDE.md ルール

1. `/rev` で変更をレビュー → Critical/Warning は `/dev` で修正
2. `docs/*.md` はコード変更時に常に最新化
3. `client/` 変更後は必ず `cd client && bunx vite build` で成功確認
