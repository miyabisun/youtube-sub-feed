# youtube-sub-feed

> English documentation: [README.md](./README.md)

YouTube の登録チャンネルの最新動画を、レコメンドアルゴリズムなしで時系列に閲覧する個人用 Web アプリ。

## 技術スタック

- **バックエンド**: Rust (axum + tokio)
- **データベース**: SQLite (rusqlite)
- **フロントエンド**: Svelte 5 + Vite
- **通知**: Discord Webhook

## 前提条件

- [Rust](https://rustup.rs/)（stable）
- [Node.js](https://nodejs.org/) v22 以上（フロントエンドビルド用）
- YouTube チャンネルを登録している Google アカウント

## セットアップ

### 1. Google Cloud プロジェクトの作成

YouTube Data API を有効にし、OAuth 2.0 クライアント認証情報を設定した Google Cloud プロジェクトが必要です。

1. [Google Cloud Console](https://console.cloud.google.com/) にアクセス
2. 新しいプロジェクトを作成（または既存のプロジェクトを選択）
3. **API とサービス > ライブラリ** に移動
4. **YouTube Data API v3** を検索し、**有効にする** をクリック
5. 左メニューの **Google Auth platform** を開き、OAuth 同意画面を設定
   1. **ブランディング**: アプリ名（例: "youtube-sub-feed"）とサポートメールを入力
   2. **対象**: **外部** を選択 → テストユーザーに自分の Google メールアドレスを追加
   3. **データアクセス**: **スコープを追加または削除** から `https://www.googleapis.com/auth/youtube.readonly` を追加
6. 左メニューの **API とサービス > 認証情報** に移動
7. **認証情報を作成 > OAuth クライアント ID** をクリック
   - アプリケーションの種類: **ウェブ アプリケーション**
   - 名前: 任意（例: "youtube-sub-feed"）
   - 承認済みの JavaScript 生成元: （空欄のままで OK）
   - 承認済みのリダイレクト URI: `http://localhost:3000/api/auth/callback` を追加
8. **クライアント ID** と **クライアント シークレット** をコピー

> **注意:** 同意画面のステータスが「テスト」の間は、追加したテストユーザーのみがログインできます。個人利用であればこのままで問題ありません。

### 2. 設定

`.env.example` を `.env` にコピーし、認証情報を入力：

```env
PORT=3000
DATABASE_PATH=./feed.db
GOOGLE_CLIENT_ID=your-client-id.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET=your-client-secret
GOOGLE_REDIRECT_URI=http://localhost:3000/api/auth/callback
```

### 3. サーバーの起動

```bash
# 開発（フロントエンドのホットリビルド付き）
./bin/dev

# — または —

# 本番
cd client && npm install && npx vite build && cd ..
cargo build --release
./target/release/youtube-sub-feed
```

`http://localhost:3000` を開き、「Google でログイン」をクリックして認可します。登録チャンネルの同期と動画の取得が自動的に開始されます。

### 4. Discord 通知（オプション）

新しい動画が検出されたときに Discord 通知を受け取るための設定：

1. Discord サーバーの **サーバー設定 > 連携サービス > ウェブフック** を開く
2. **新しいウェブフック** をクリックし、チャンネルを選択して **ウェブフック URL をコピー**

`.env` に追加：

```env
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/xxx/xxx
```

サーバーを再起動すると、ポーリングで新着動画が検出されるたびに Embed が送信されます。

## Docker

```bash
docker build -t youtube-sub-feed .

docker run -d \
  --name youtube-sub-feed \
  -p 3000:3000 \
  -v ytfeed-data:/app \
  --env-file .env \
  youtube-sub-feed
```

本番環境では、`.env` の `GOOGLE_REDIRECT_URI` を実際のドメインに合わせて変更し（例: `https://feed.example.com/api/auth/callback`）、同じ URI を Google Cloud Console の承認済みリダイレクト URI にも追加してください。

## 仕組み

- 初回ログイン時に YouTube の登録チャンネルをすべて同期し、最新の動画を取得
- バックグラウンドで 2 つのポーリングループが稼働：
  - **新着検知**（15 分/周）: `show_livestreams=0` の全チャンネルを RSS-First 戦略で巡回 — RSS で新着を検知したチャンネルのみ YouTube API を呼び出し
  - **ライブ察知**（5 分/周）: `show_livestreams=1` のチャンネルのみ API 直叩きで巡回 + ライブ終了検知
- 登録チャンネルリストは 10 分ごとに同期
- 動画はグループで整理、スワイプで非表示、種別（ショート・ライブ配信）でフィルタ可能

## 環境変数

| 変数 | デフォルト値 | 説明 |
|------|-------------|------|
| `PORT` | `3000` | サーバーのポート番号 |
| `DATABASE_PATH` | `./feed.db` | SQLite データベースファイルのパス |
| `GOOGLE_CLIENT_ID` | — | Google OAuth クライアント ID（必須） |
| `GOOGLE_CLIENT_SECRET` | — | Google OAuth クライアントシークレット（必須） |
| `GOOGLE_REDIRECT_URI` | `http://localhost:3000/api/auth/callback` | OAuth コールバック URL |
| `DISCORD_WEBHOOK_URL` | — | Discord Webhook URL（オプション） |

## コマンド

| コマンド | 説明 |
|---------|------|
| `./bin/dev` | 開発サーバー起動（フロントエンドのホットリビルド付き） |
| `cargo build --release` | 本番ビルド |
| `cargo test` | 全テスト実行 |
