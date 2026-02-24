# Rust 移行ガイド

novel-server の Bun → Rust 移行（2026-02）の実績に基づく、youtube-sub-feed 向けの移行ガイド。

## 動機

- Bun 1インスタンスあたり RSS ~130-140MB → Rust で ~30-40MB に削減（novel-server 実績）
- 1GB VPS で多くのインスタンスを運用可能に

## 技術スタックの対応表

| 用途 | 現行 (Bun/TypeScript) | 移行先 (Rust) | 備考 |
|------|----------------------|---------------|------|
| ランタイム | Bun | tokio | |
| Web フレームワーク | Hono | axum | ルーティングが Hono に近い |
| DB | bun:sqlite (raw) | rusqlite (bundled) | |
| HTTP クライアント | fetch | reqwest | **カンマエンコード注意** |
| OAuth2 | 自前実装 | reqwest 直接実装 | oauth2 crate 不使用、エンドポイント直叩き |
| セッション | Cookie + DB | Cookie + DB（同構造） | `tower-cookies` で Cookie 操作 |
| Discord 通知 | discord.js (Bot) | Webhook に変更推奨 | Bot→Webhook で依存を大幅削減 |
| RSS パース | — (fetch + XML) | regex-lite による自前パース | feed-rs 不使用、Atom XML を正規表現で抽出 |
| 日時 | Date / ISO 文字列 | chrono | ISO 8601 パース対応 |
| 環境変数 | process.env | dotenvy | |
| シリアライズ | JSON.parse/stringify | serde + serde_json | |
| ログ | console.log | tracing + tracing-subscriber | |
| エラー型 | throw + try/catch | thiserror + AppError enum | |
| フロントエンド | Svelte 5 + Vite | Svelte 5 + Vite（変更なし） | |
| フロントエンドビルド | Bun (bunx) | Node.js (npx) | |

## novel-server との差分（追加の考慮事項）

### 1. OAuth2 認証

novel-server は認証なし。youtube-sub-feed には Google OAuth2 + セッション管理がある。

**`oauth2` crate の採用**:
```rust
// Authorization Code Flow
let client = BasicClient::new(client_id, Some(client_secret), auth_url, Some(token_url));
let (auth_url, csrf_token) = client
    .authorize_url(CsrfToken::new_random)
    .add_scope(Scope::new("https://www.googleapis.com/auth/youtube.readonly".into()))
    .url();
```

**セッション管理**:
- `tower-cookies` で HttpOnly Cookie の読み書き
- セッションID → DB 照合は現行と同じ構造
- `Secure` フラグの本番/開発切り替えは `Config` から制御

**認証ミドルウェア**:
```rust
// axum の middleware::from_fn で認証チェック
async fn auth_middleware(
    State(state): State<AppState>,
    cookies: Cookies,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    // /api/auth/login, /api/auth/callback はスキップ
    // Cookie からセッションID取得 → DB検証 → 期限チェック
}
```

### 2. YouTube Data API

reqwest での API 呼び出しで最も注意すべき点:

**カンマのエンコード問題（novel-server で発覚）**:
```rust
// NG: reqwest が part=snippet,contentDetails を part=snippet%2CcontentDetails にエンコード
let resp = client.get(url).query(&[("part", "snippet,contentDetails")]).send().await?;

// OK: クエリ文字列を URL に直接埋め込む
let url = format!(
    "https://www.googleapis.com/youtube/v3/videos?part={}&id={}",
    "snippet,contentDetails,liveStreamingDetails",
    video_ids.join(",")
);
let resp = client.get(&url).send().await?;
```

YouTube API の `part` パラメータや `id` パラメータはカンマ区切りが必須。`reqwest::RequestBuilder::query()` を使うとカンマが `%2C` にエンコードされ、API がエラーを返す。**URL に直接埋め込む**こと。

**Videos.list のバッチ取得**:
- 最大50件/リクエストの制約は同じ
- `futures::future::join_all` で並行リクエストも可能だが、クォータ考慮で逐次が安全

### 3. Discord 通知

discord.js (Bot) → **Discord Webhook** への変更を推奨。

**理由**:
- discord.js はエコシステム全体が JS 依存。Rust の Discord Bot ライブラリ（serenity 等）は巨大な依存ツリーを持つ
- Webhook なら reqwest の POST 1つで完結。追加 crate 不要
- 送信専用（受信不要）の用途に Bot は過剰

```rust
// Webhook で Embed 送信
let payload = json!({
    "embeds": [{
        "author": { "name": channel_name },
        "title": video_title,
        "url": video_url,
        "image": { "url": thumbnail_url },
        "timestamp": published_at,
        "color": 0xd93025
    }]
});
client.post(&webhook_url).json(&payload).send().await?;
```

**環境変数の変更**:
- `DISCORD_TOKEN` + `DISCORD_CHANNEL_ID` → `DISCORD_WEBHOOK_URL` に一本化

### 4. DB スキーマ（6テーブル）

novel-server は 1 テーブルだったが、youtube-sub-feed は 6 テーブル + FK + CASCADE。

**rusqlite での FK 有効化**:
```rust
conn.execute_batch("
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    PRAGMA foreign_keys = ON;  -- 重要: デフォルトは OFF
    PRAGMA cache_size = -64000;
    PRAGMA temp_store = MEMORY;
")?;
```

**CASCADE 削除**: rusqlite は `PRAGMA foreign_keys = ON` で FK 制約と CASCADE が有効になる。novel-server では不要だったが、youtube-sub-feed では必須。

**トランザクション**:
```rust
// 巡回結果の DB 書き込みは短いトランザクションで実行
fn upsert_videos(conn: &Connection, videos: &[Video]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for video in videos {
        tx.execute("INSERT INTO videos ... ON CONFLICT(id) DO UPDATE SET ...", params![...])?;
    }
    tx.commit()?;
    Ok(())
}
```

### 5. 巡回ループ（2本並行）

novel-server は 3 ループ（narou, nocturne, kakuyomu）。youtube-sub-feed は 2 ループ（新着検知15分 + ライブ察知5分）。

**構造は同じ**: `tokio::spawn` + `tokio::time::sleep` チェーン。

```rust
pub fn start_sync(state: AppState) {
    // 新着検知 (15分/周, RSS-First) + ライブ察知 (5分/周, API直叩き)
    polling::start_polling(state.clone());
    // 登録チャンネル同期 (10分 interval)
    subscriptions::start_periodic_sync(state);
}
```

**クォータ超過時の待機**:
```rust
// 太平洋時間 午前0時まで待機
if is_quota_exceeded(&err) {
    let now = Utc::now();
    let pacific_midnight = next_pacific_midnight(now);
    let wait = (pacific_midnight - now).to_std().unwrap();
    tracing::warn!("Quota exceeded, waiting until {}", pacific_midnight);
    tokio::time::sleep(wait).await;
}
```

### 6. RSS パース

youtube-sub-feed 固有の機能。YouTube の Atom フィードは構造が単純なため、`regex-lite` による軽量パースを採用（`feed-rs` 等の外部 crate は不使用）。

```rust
use regex_lite::Regex;

let entry_re = Regex::new(r"<entry>([\s\S]*?)</entry>").unwrap();
let video_id_re = Regex::new(r"<yt:videoId>([^<]+)</yt:videoId>").unwrap();

for cap in entry_re.captures_iter(&xml) {
    let video_id = video_id_re.captures(&cap[1]).map(|c| c[1].to_string());
    // ...
}
```

## 移行フェーズ

### Phase 0: プロジェクト初期化

```
Cargo.toml
src/
  main.rs       — axum Router + tokio::main
  config.rs     — 環境変数
  error.rs      — AppError enum
  state.rs      — AppState
```

### Phase 1: 基盤

```
src/
  db.rs         — rusqlite + PRAGMA + 6テーブル CREATE
  cache.rs      — HashMap TTL キャッシュ（novel-server と同構造）
  spa.rs        — SPA 配信 + BASE_PATH
```

### Phase 2: 認証

```
src/
  auth.rs       — OAuth2 フロー + トークンリフレッシュ
  session.rs    — セッション CRUD + Cookie 操作
  middleware.rs — 認証ミドルウェア
```

認証が動けばフロントエンドのログインフローを通しで確認できる。

### Phase 3: YouTube API クライアント

```
src/
  youtube/
    mod.rs       — API クライアント共通（認証ヘッダー、リトライ、クォータ検知）
    subscriptions.rs — Subscriptions.list
    playlist_items.rs — PlaylistItems.list
    videos.rs    — Videos.list
```

**テスト必須**: `part` パラメータのカンマがエンコードされないことを確認するテスト。

### Phase 4: API ルート

```
src/
  routes/
    mod.rs
    auth.rs       — login, callback, logout, me
    feed.rs       — 動画フィード
    videos.rs     — hide/unhide
    channels.rs   — 一覧, 動画, sync, refresh, 設定更新
    groups.rs     — CRUD, reorder, channels 割り当て
```

### Phase 5: バックグラウンド処理

```
src/
  sync/
    mod.rs            — start_sync + wait_for_token/wait_for_quota 共通関数
    polling.rs        — 新着検知 (15分/周, RSS-First) + ライブ察知 (5分/周, API直叩き)
    subscriptions.rs  — チャンネルリスト同期 (10分 interval)
    token.rs          — アクセストークン取得・リフレッシュ
    video_fetcher.rs  — チャンネル動画取得（UPSERT, shorts判定, 通知）
    rss_checker.rs    — RSS-First 新着判定
    channel_sync.rs   — リモート/ローカル差分同期
    initial_setup.rs  — 初回セットアップフロー
    livestream.rs     — ライブ終了検知
```

### Phase 6: Discord 通知

```
src/
  notify.rs     — Webhook POST（Embed 形式）
```

### Phase 7: 統合・Docker

- `Dockerfile` マルチステージビルド（novel-server と同構造）
- `bin/dev` 開発スクリプト
- ドキュメント更新

## ハマりポイント（novel-server 実績）

### 1. `Send` 境界と非同期（最大の壁）

`tokio::spawn` 内で rusqlite の `Statement` や `MappedRows` が `.await` を跨ぐとコンパイルエラー。

```rust
// NG: Mutex guard が .await を跨ぐ
let data = {
    let conn = state.db.lock().unwrap();
    conn.prepare("SELECT ...")?.query_map([], |row| ...)?  // MappedRows は Send ではない
        .collect()
};
some_async_fn().await;  // Send 境界違反

// OK: 同期関数に切り出す
fn get_channels(conn: &Connection) -> Result<Vec<Channel>> { /* DB操作を完結 */ }
let channels = { let conn = state.db.lock().unwrap(); get_channels(&conn)? };
some_async_fn().await;  // OK
```

### 2. reqwest のカンマエンコード

**YouTube API で致命的**。`part=snippet,contentDetails` が `part=snippet%2CcontentDetails` になり API がエラーを返す。コンパイラでは検出不可能。

**対策**: URL 文字列を直接構築し、`.query()` メソッドを使わない。パラメータ形式のテストを書く。

### 3. Docker ビルドの OpenSSL 依存

`rust:1-slim` に OpenSSL ヘッダーがない。reqwest (native-tls) を使う場合:

```dockerfile
FROM rust:1-slim AS backend
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
```

## Cargo.toml 依存関係

```toml
[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["full"] }
rusqlite = { version = "0.33", features = ["bundled"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
dotenvy = "0.15"
tracing = "0.1"
tracing-subscriber = "0.3"
thiserror = "2"
tower-http = { version = "0.6", features = ["fs", "trace"] }
tower-cookies = "0.11"
tower = { version = "0.5", features = ["util"] }
uuid = { version = "1", features = ["v4"] }
regex-lite = "0.1"
urlencoding = "2"
```

## 移行して良かった点（novel-server 実績）

- メモリ消費が 1/4 以下に（130-140MB → 30-40MB）
- axum のルーティングが Hono に近く移植しやすい
- `thiserror` + `AppError` enum による型安全なエラーハンドリング
- コンパイル時にほとんどのバグを検出できる（reqwest のカンマ問題は例外）
