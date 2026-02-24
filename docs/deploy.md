# デプロイ手順

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
  -e GOOGLE_CLIENT_ID=xxx \
  -e GOOGLE_CLIENT_SECRET=xxx \
  -e GOOGLE_REDIRECT_URI=https://feed.sis.jp/api/auth/callback \
  -e DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/xxx/xxx \
  youtube-sub-feed
```

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
| `GOOGLE_CLIENT_ID` | Google OAuth2 クライアントID |
| `GOOGLE_CLIENT_SECRET` | Google OAuth2 クライアントシークレット |
| `GOOGLE_REDIRECT_URI` | OAuth2 コールバックURL |
| `DISCORD_WEBHOOK_URL` | Discord Webhook URL（省略可） |
