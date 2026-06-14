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
- Cloudflare アカウント（入口認証に Cloudflare Access を使用）

## セットアップ

### 1. Google Cloud プロジェクトの作成（チャンネル同期ボタンを使う場合のみ）

ヘッダーメニューの「チャンネル同期 (YouTube)」ボタンを使う場合、GIS クライアント ID が必要です。
手動でチャンネル ID を追加するだけの場合はスキップできます。

1. [Google Cloud Console](https://console.cloud.google.com/) にアクセス
2. 新しいプロジェクトを作成（または既存のプロジェクトを選択）
3. **API とサービス > ライブラリ** に移動し、**YouTube Data API v3** を有効にする
4. 左メニューの **Google Auth platform** を開き、OAuth 同意画面を設定
   - **対象**: **外部** を選択 → テストユーザーに自分の Google メールアドレスを追加
   - **データアクセス**: `https://www.googleapis.com/auth/youtube.readonly` を追加
5. 左メニューの **API とサービス > 認証情報** に移動
6. **認証情報を作成 > OAuth クライアント ID** をクリック
   - アプリケーションの種類: **ウェブ アプリケーション**
   - 承認済みの JavaScript 生成元: `http://localhost:3000`（開発時）
7. 作成した **クライアント ID** を `GIS_CLIENT_ID` に設定する

> **注意:** クライアントシークレットは不要です。ブラウザ側の GIS（Google Identity Services）が短命トークンを取得し、サーバーにトークンを送信・保存しません。テストユーザーに自分を追加すれば「未確認アプリ」警告を経由して利用できます。

### 2. 設定

`.env.example` を `.env` にコピーし、設定を入力：

```env
PORT=3000
DATABASE_PATH=./feed.db
GIS_CLIENT_ID=your-client-id.apps.googleusercontent.com
WEBSUB_CALLBACK_URL=http://localhost:3000/api/websub/callback
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

`http://localhost:3000` を開きます。開発環境では最初の DB ユーザーが自動的に認証されます（devbypass）。本番では Cloudflare Access が入口を担当します。

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

本番環境では Cloudflare Access を前段に設置してください。詳細は `docs/deploy.md` を参照してください。

## 仕組み

- チャンネルは手動登録（チャンネル ID 直接入力）またはヘッダーメニューの「チャンネル同期 (YouTube)」で一括取込
- 登録時に WebSub (PubSubHubbub) サブスクリプションを自動設定し、新着動画をプッシュ通知で受信
- バックグラウンドで WebSub push を主軸に動作：新着検知は Google API 呼び出しゼロ
- 動画はグループで整理、スワイプで非表示、種別（ショート・ライブ配信）でフィルタ可能

## 環境変数

| 変数 | デフォルト値 | 説明 |
|------|-------------|------|
| `PORT` | `3000` | サーバーのポート番号 |
| `DATABASE_PATH` | `./feed.db` | SQLite データベースファイルのパス |
| `GIS_CLIENT_ID` | — | Google Identity Services クライアント ID（チャンネル同期ボタン用。公開値、シークレット不要） |
| `WEBSUB_CALLBACK_URL` | `http://localhost:3000/api/websub/callback` | WebSub 通知受信エンドポイント（本番は公開 HTTPS URL 必須） |
| `DISCORD_WEBHOOK_URL` | — | Discord Webhook URL（オプション） |

## コマンド

| コマンド | 説明 |
|---------|------|
| `./bin/dev` | 開発サーバー起動（フロントエンドのホットリビルド付き） |
| `cargo build --release` | 本番ビルド |
| `cargo test` | 全テスト実行 |
