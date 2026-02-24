# novel-server 設計リファレンス

youtube-sub-feed が準拠する、novel-server（同一マシン上の別プロジェクト）の設計方針・規約の抜粋。
実装時に novel-server のソースコードを直接読む許可あり。

## プロジェクト構成

```
novel-server/
├── Cargo.toml               # Rust プロジェクト定義
├── src/                     # バックエンド (Rust)
│   ├── main.rs              # エントリポイント (axum + tokio::main)
│   ├── config.rs            # 環境変数の読み込み
│   ├── error.rs             # AppError enum + IntoResponse
│   ├── state.rs             # AppState (db, cache, config, http)
│   ├── db.rs                # SQLite 初期化 (rusqlite)
│   ├── cache.rs             # インメモリキャッシュ (HashMap + TTL)
│   ├── sanitize.rs          # HTML サニタイズ (ammonia)
│   ├── spa.rs               # SPA インデックス配信 & BASE_PATH 対応
│   ├── sync.rs              # バックグラウンド定期同期
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
├── bin/dev                  # ローカル開発用スクリプト (ビルド＆起動)
├── docs/                    # ドキュメント
├── CLAUDE.md                # プロジェクト固有ルール
└── Dockerfile               # マルチステージビルド
```

## 技術スタック

> **注意**: 2026-02 に Bun/TypeScript から Rust に移行済み。
> youtube-sub-feed は Bun/TypeScript のまま運用しているため、直接的な技術スタックの参照先ではなくなった。
> ただしアーキテクチャパターン（キャッシュ・同期・SPA配信等）とフロントエンド規約は引き続き有効。

| 項目 | 旧 (TypeScript) | 現行 (Rust) |
|------|-----------------|-------------|
| ランタイム | Bun | tokio |
| フレームワーク | Hono | axum |
| DB | bun:sqlite + Drizzle | rusqlite (bundled) |
| フロントエンド | Svelte 5 + Vite | Svelte 5 + Vite（変更なし） |
| フロントエンドビルド | Bun (bunx) | Node.js (npx) |
| スタイリング | Sass | Sass（変更なし） |
| HTML パース | cheerio | scraper |
| HTML サニタイズ | HTMLRewriter | ammonia |
| HTTP クライアント | fetch | reqwest |
| ログ | Hono logger | tracing |
| エラー型 | throw + try/catch | thiserror + AppError enum |

## Rust 移行の知見

youtube-sub-feed で同様の移行を検討する場合の参考情報。

### 移行の動機

- Bun の1インスタンスあたり RSS ~130-140MB → Rust で ~30-40MB に削減
- 1GB VPS で多くのインスタンスを運用可能に

### ハマりポイント

#### 1. `Send` 境界と非同期 (最大の壁)

`tokio::spawn` 内で rusqlite の `Statement` や `MappedRows` が `.await` を跨ぐとコンパイルエラー。DB 操作は同期関数に切り出し `{}` ブロック内で完結させる必要がある。

```rust
// NG: Mutex guard が .await を跨ぐ
let ids = {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare("SELECT id FROM ...")?;
    stmt.query_map([], |row| row.get(0))? // MappedRows は Send ではない
        .collect()
};
some_async_fn().await; // ← ここで Send 境界違反

// OK: 同期関数に切り出す
fn get_ids(conn: &Connection) -> Vec<String> { /* DB操作を完結 */ }
let ids = { let conn = state.db.lock().unwrap(); get_ids(&conn) };
some_async_fn().await; // OK
```

#### 2. 外部 API パラメータのサイレント破壊

reqwest はクエリパラメータのカンマを `%2C` にエンコードする。TypeScript の `fetch` では問題なかったパラメータ形式が Rust で壊れ、API がデータの代わりに null を返した。**コンパイラでは検出不可能**で、curl での手動テストで発見。

**対策**: API パラメータ形式は定数化し、フォーマットのテストを書く。

#### 3. Docker ビルドの依存

`rust:1-slim` には OpenSSL の開発ヘッダーがない。reqwest (native-tls) を使う場合、Dockerfile に `pkg-config` と `libssl-dev` のインストールが必要。

### 移行して良かった点

- メモリ消費が 1/4 以下に
- scraper (HTML パース) と ammonia (サニタイズ) は cheerio/HTMLRewriter とほぼ同じ感覚
- axum のルーティングが Hono に近く移植しやすい

## DB 初期化パターン (src/db.rs)

```rust
// rusqlite でオープン + PRAGMA 設定
let conn = Connection::open(path)?;
conn.execute_batch("
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    PRAGMA cache_size = -64000;
    PRAGMA temp_store = MEMORY;
")?;
// CREATE TABLE IF NOT EXISTS ...
```

## インメモリキャッシュ (src/cache.rs)

- `HashMap<String, CacheEntry>` ベースの TTL キャッシュ
- 最大 10,000 エントリ
- 1時間ごとにスイープ
- FIFO で最古エントリを削除

## バックグラウンド同期パターン (src/sync.rs)

2種類の同期パターンを使用:

### 一括同期 (なろう・ノクターン)
```
tokio::time::interval(10分)
→ fetch_data() で全件一括取得 → DB一括更新
```

### ラウンドロビン同期 (カクヨム)
```
loop {
    1件ずつ fetch_datum() → DB更新
    tokio::time::sleep(3_600_000ms / count)
}
// 1時間で全件を均等に循環
```

## SPA 配信 (src/spa.rs)

- `client/build/` の静的ファイルを `tower_http::services::ServeDir` で配信
- SPA のフォールバック: 未マッチのルートに `index.html` を返す
- `BASE_PATH` 対応: `<base>` タグと `window.__BASE_PATH__` を動的注入

## API ルートの規約

```rust
// エラーレスポンス
(StatusCode::BAD_REQUEST, Json(json!({"error": "Invalid type"})))
(StatusCode::BAD_GATEWAY, Json(json!({"error": "Failed to fetch ranking"})))

// リトライパターン (最大3回、線形バックオフ)
for attempt in 0..3 {
    match fetch().await {
        Ok(data) => return Ok(data),
        Err(e) if attempt < 2 => {
            tokio::time::sleep(Duration::from_millis(500 * (attempt + 1) as u64)).await;
        }
        Err(e) => return Err(e),
    }
}
```

## フロントエンド規約

> フロントエンドは Rust 移行後も変更なし。

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
# Stage 1: Frontend
FROM node:22-slim
# npm ci → npx vite build

# Stage 2: Rust build
FROM rust:1-slim
# apt-get install pkg-config libssl-dev
# cargo build --release

# Stage 3: Production
FROM debian:bookworm-slim
# ca-certificates のみインストール
# バイナリ + client/build/ をコピー
# CMD: ["novel-server"]
```

## CLAUDE.md ルール

1. `/rev` で変更をレビュー → Critical/Warning は `/dev` で修正
2. `docs/*.md` はコード変更時に常に最新化
3. `client/` 変更後は必ず `cd client && npx vite build` で成功確認
