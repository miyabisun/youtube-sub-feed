# youtube-sub-feed

YouTube の登録チャンネルの最新動画を、レコメンドアルゴリズムなしで時系列に閲覧する個人用 Web アプリ。

## 前提条件

- [Bun](https://bun.sh/) v1.0 以上
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

### 2. インストールと設定

```bash
bun run setup
```

依存関係がインストールされ、`.env.example` から `.env` が作成されます。`.env` を編集：

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
bun run dev

# — または —

# 本番
bun run build
bun start
```

`http://localhost:3000` を開き、「Google でログイン」をクリックして認可します。登録チャンネルの同期と動画の取得が自動的に開始されます。

### 4. Discord 通知（オプション）

新しい動画が検出されたときに Discord 通知を受け取るための設定：

1. [Discord Developer Portal](https://discord.com/developers/applications) にアクセス
2. 新しいアプリケーションを作成し、**Bot** タブを開く
3. **Reset Token** をクリックしてボットトークンをコピー
4. Privileged Gateway Intents で **MESSAGE CONTENT INTENT** を有効化（厳密には不要だが推奨）
5. **OAuth2 > URL Generator** を開く
   - Scopes: `bot`
   - Bot Permissions: `Send Messages`, `Embed Links`
   - 生成された URL を開いてボットをサーバーに招待
6. Discord で開発者モードを有効化（設定 > 詳細設定）し、通知先チャンネルを右クリックして **チャンネル ID をコピー**

`.env` に追加：

```env
DISCORD_TOKEN=your-bot-token
DISCORD_CHANNEL_ID=your-channel-id
```

サーバーを再起動すると、ポーリングで新着動画が検出されるたびに Embed が送信されます。

## Docker

`main` ブランチへのプッシュごとに、ビルド済みイメージが GitHub Container Registry に公開されます。

```bash
docker pull ghcr.io/miyabisun/youtube-sub-feed:latest

docker run -d \
  --name youtube-sub-feed \
  -p 3000:3000 \
  -v ytfeed-data:/app \
  --env-file .env \
  -e NODE_ENV=production \
  ghcr.io/miyabisun/youtube-sub-feed:latest
```

ローカルでビルドする場合：

```bash
docker build -t youtube-sub-feed .

docker run -d \
  --name youtube-sub-feed \
  -p 3000:3000 \
  -v ytfeed-data:/app \
  --env-file .env \
  -e NODE_ENV=production \
  youtube-sub-feed
```

本番環境では、`.env` の `GOOGLE_REDIRECT_URI` を実際のドメインに合わせて変更し（例: `https://feed.example.com/api/auth/callback`）、同じ URI を Google Cloud Console の承認済みリダイレクト URI にも追加してください。

## 仕組み

- 初回ログイン時に YouTube の登録チャンネルをすべて同期し、最新の動画を取得
- バックグラウンドで 2 つのポーリングループが稼働：
  - **通常巡回**（30 分/周）: 全チャンネルをローテーション
  - **高頻度巡回**（10 分/周）: 「高頻度巡回」に設定したチャンネルのみ
- 1 日 1 回の同期でチャンネルの登録・解除を反映
- 動画はグループで整理、スワイプで非表示、種別（ショート・ライブ配信）でフィルタ可能

## 環境変数

| 変数 | デフォルト値 | 説明 |
|------|-------------|------|
| `PORT` | `3000` | サーバーのポート番号 |
| `DATABASE_PATH` | `./feed.db` | SQLite データベースファイルのパス |
| `GOOGLE_CLIENT_ID` | — | Google OAuth クライアント ID（必須） |
| `GOOGLE_CLIENT_SECRET` | — | Google OAuth クライアントシークレット（必須） |
| `GOOGLE_REDIRECT_URI` | `http://localhost:3000/api/auth/callback` | OAuth コールバック URL |
| `DISCORD_TOKEN` | — | Discord ボットトークン（オプション） |
| `DISCORD_CHANNEL_ID` | — | 通知先の Discord チャンネル ID（オプション） |

## コマンド

| コマンド | 説明 |
|---------|------|
| `bun run setup` | 依存関係のインストールと `.env` の作成 |
| `bun run dev` | 開発サーバー起動（フロントエンドのホットリビルド付き） |
| `bun run build` | フロントエンドの本番ビルド |
| `bun start` | 本番サーバー起動 |
| `bun test` | 全テスト実行 |
