# デプロイ手順

## 前提: Cloudflare Access の設定

このアプリは認証に **Cloudflare Access** を使用します。
Cloudflare Zero Trust でアプリケーションを作成し、アクセスポリシーを設定してください。
Cloudflare Access は認証済みリクエストに `Cf-Access-Authenticated-User-Email` ヘッダーを付与します。
このヘッダーをサーバーが読み取り、ユーザーを特定します。

> **セキュリティ重要**: アプリの **3000 番ポートを外部から直接到達可能な状態にしないこと**。
> 必ず Cloudflare Tunnel / Cloudflare Access 経由でのみアクセスできるように設定してください。
> `Cf-Access-Authenticated-User-Email` ヘッダーはアプリ側で無検証で信頼するため、
> ポートが直接公開されるとヘッダー偽装で任意ユーザーになりすませます。

## Docker ビルド

```bash
docker build -t youtube-sub-feed .
```

## Docker 起動

```bash
docker run -d \
  --name youtube-sub-feed \
  -p 3000:3000 \
  -v /path/to/data:/data \
  -e DATABASE_PATH=/data/feed.db \
  -e GIS_CLIENT_ID=xxx.apps.googleusercontent.com \
  -e YOUTUBE_API_KEY=AIzaXXXX \
  -e WEBSUB_CALLBACK_URL=https://feed.sis.jp/api/websub/callback \
  -e PUBLIC_BASE_URL=https://feed.sis.jp \
  -e DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/xxx/xxx \
  youtube-sub-feed
```

WebSub (PubSubHubbub) 経由で YouTube から新着動画のプッシュ通知を受信するため、`WEBSUB_CALLBACK_URL` には **公開 HTTPS URL** を指定する必要があります。

## GIS_CLIENT_ID の設定

`GIS_CLIENT_ID` は Google Identity Services (GIS) のクライアント ID です。
ブラウザのチャンネル同期機能で使用します（サーバーからの YouTube API 呼び出しには
`YOUTUBE_API_KEY` を使用し、OAuth トークンは扱いません）。

1. [Google Cloud Console](https://console.cloud.google.com/) でプロジェクトを作成
2. 「APIとサービス」→「認証情報」→「OAuth 2.0 クライアントID」を作成
   - アプリケーションの種類: **ウェブアプリケーション**
   - 承認済みの JavaScript 生成元: `https://feed.sis.jp` など
3. 作成したクライアント ID を `GIS_CLIENT_ID` に設定

注意: このクライアント ID はブラウザの JS に埋め込まれる公開値です（シークレットではありません）。
サーバー側にアクセストークンは送信・保存されません。

## YOUTUBE_API_KEY の設定

`YOUTUBE_API_KEY` は YouTube Data API v3 の API キーです。動画詳細
（再生時間・プレーヤー寸法による Shorts 判定・ライブ配信状態）のエンリッチに使用します。
未設定でも動作しますが、Shorts フィルタ（ショートNG）とライブ判定が機能しません。

1. [Google Cloud Console](https://console.cloud.google.com/) で「APIとサービス」→「ライブラリ」から **YouTube Data API v3** を有効化
2. 「認証情報」→「認証情報を作成」→「APIキー」を作成
3. 作成したキーを `YOUTUBE_API_KEY` に設定（「APIの制限」で YouTube Data API v3 のみに絞ることを推奨）

消費クォータは新着動画 1 push あたり 1 ユニットです。再生時間・プレーヤー寸法・
ライブ配信状態は同じ `videos.list` リクエストで取得するため、無料枠 10,000
ユニット/日に対して十分小さい値です。

## 初回セットアップ

1. コンテナ起動後、Cloudflare Access 経由で最初にアクセスしたユーザーが **マスターユーザー** として自動登録されます。
2. ヘッダーメニューの「チャンネル同期 (YouTube)」から Google アカウントを認可してチャンネルを同期、
   または「チャンネル」ページから UC で始まるチャンネル ID を直接入力して手動追加できます。
3. チャンネルを追加すると WebSub サブスクリプションが自動的に登録され、新着動画がプッシュ通知されます。

## nginx 設定例

```nginx
server {
    listen 443 ssl http2;
    server_name feed.sis.jp;

    ssl_certificate /etc/letsencrypt/live/feed.sis.jp/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/feed.sis.jp/privkey.pem;

    location / {
        proxy_pass http://localhost:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}

server {
    listen 80;
    server_name feed.sis.jp;
    return 301 https://$host$request_uri;
}
```

## Volume マウント

- `/data/feed.db` — SQLite データベースファイル
- コンテナ再起動時もデータが永続化される

## 環境変数

| 変数 | 説明 |
|------|------|
| `PORT` | サーバーポート (デフォルト: 3000) |
| `DATABASE_PATH` | SQLite DBファイルパス |
| `GIS_CLIENT_ID` | Google Identity Services クライアント ID（ブラウザ側チャンネル同期に使用） |
| `YOUTUBE_API_KEY` | YouTube Data API v3 の API キー（動画詳細エンリッチに使用、省略可） |
| `WEBSUB_CALLBACK_URL` | WebSub 通知受信エンドポイント（例: `https://feed.sis.jp/api/websub/callback`）。公開 HTTPS URL 必須 |
| `PUBLIC_BASE_URL` | JSON Feedなどのフィード内リンクに使う公開オリジン（例: `https://feed.sis.jp`） |
| `DISCORD_WEBHOOK_URL` | Discord Webhook URL（省略可） |

## 削除された環境変数（旧 Google OAuth）

以下の環境変数は不要になりました。旧バージョンから移行する場合は削除してください:

- `GOOGLE_CLIENT_ID` — Google OAuth2 クライアントID（不要）
- `GOOGLE_CLIENT_SECRET` — Google OAuth2 クライアントシークレット（不要）
- `GOOGLE_REDIRECT_URI` — OAuth2 コールバックURL（不要）
