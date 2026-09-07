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
PUBLIC_BASE_URL=https://youtube.example.com
```

### 3. サーバーの起動

```bash
# 開発（フロントエンドのホットリビルド付き）
./bin/dev

# — または —

# 本番
cd client && npm install && npx vite build && cd ..
cargo build --release
NODE_ENV=production ./target/release/youtube-sub-feed
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
  -e NODE_ENV=production \
  youtube-sub-feed
```

本番環境では Cloudflare Access を前段に設置してください。詳細は `docs/deploy.md` を参照してください。

## 仕組み

- チャンネルは手動登録（チャンネル ID 直接入力）またはヘッダーメニューの「チャンネル同期 (YouTube)」で一括取込
- 登録時に WebSub (PubSubHubbub) サブスクリプションを自動設定し、新着動画をプッシュ通知で受信
- バックグラウンドで WebSub push を主軸に動作：新着検知は Google API 呼び出しゼロ
- 動画はグループで整理、スワイプで非表示、種別（ショート・ライブ配信）でフィルタ可能

## 環境変数

現行名・必須/任意・既定値・不正値の扱いは
[README.md の Environment Variables](README.md#environment-variables) を正本とします。
`PORT`、`DATABASE_PATH`、`GIS_CLIENT_ID`、`WEBSUB_CALLBACK_URL`、`PUBLIC_BASE_URL`、
`DISCORD_WEBHOOK_URL`、`YOUTUBE_API_KEY`、`CATCHUP_INTERVAL_MINUTES`、`NODE_ENV`、`RUST_LOG` を扱います。

本番では `NODE_ENV=production` をサーバーの環境に明示してください。release build や
Docker イメージだけでは本番モードにならず、未設定・誤記時は開発用のユーザー fallback が有効です。
旧 `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` / `GOOGLE_REDIRECT_URI` は参照しません。
ブラウザ側の同期には `GIS_CLIENT_ID` を使い、サーバー側 secret や redirect の代替変数はありません。

## コマンド

| コマンド | 説明 |
|---------|------|
| `./bin/dev` | 開発サーバー起動（フロントエンドのホットリビルド付き） |
| `cargo build --release` | 本番ビルド |
| `cargo test` | 全テスト実行 |

### WebSub の購読確認と再試行

起動・チャンネル追加・24時間ごとの更新・手動全件取得は、申請前に Google Hub の
[公式 Subscriber Diagnostics](https://pubsubhubbub.appspot.com/subscribe) で各 topic と callback を照会します。
全購読一覧の JSON API ではなく、`GET /subscription-details` の HTML 診断です。
既存の署名 secret を使い、購読が有効で期限まで2日を超えるものは再申請しません。
診断のHTTPエラー・未知のHTML・状態・日時はログに残し、コールバックで確認済みのDB期限へフォールバックします。
その場合、Hubの現在状態を直接確認できたとは扱いません。診断結果でDBの署名secretや確認済み期限を書き換えることもありません。

全経路で同じ送信制御を使い、診断・購読・解除を10秒以上空けて送ります。
一時エラー（通信失敗、408、429、500、502、503、504）は30秒以上空けて2回まで再試行し、
`Retry-After` の秒数・HTTP日時がさらに先ならその時刻まで待ちます。恒久エラーは再試行しません。
全対象の処理後、最終失敗だけをチャンネル名でDiscordへ1回通知します。
名前が未入力・ID代用の場合だけ公開Atomフィードからチャンネル名を取得して保存し、取得不能時は「名前未取得のチャンネル」と表示します（IDはログに残します）。
起動直後の同じ失敗を定期処理で繰り返さないよう、次の購読更新は24時間後です。動画情報の補完は即時に行います。
HTTP受付は非同期確認の完了を意味せず、受付後1時間は同じチャンネルの再申請を待ちます。
購読状態にかかわらず、手動全件取得の動画スキャンは維持されます。
[YouTube の公式プッシュ通知手順](https://developers.google.com/youtube/v3/guides/push_notifications) も参照してください。
