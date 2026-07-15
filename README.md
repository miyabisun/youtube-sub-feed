# youtube-sub-feed

> 日本語ドキュメントは [README.ja.md](./README.ja.md) を参照してください。

A personal web app for browsing your YouTube subscriptions chronologically, without the recommendation algorithm.

## Tech Stack

- **Backend**: Rust (axum + tokio)
- **Database**: SQLite (rusqlite)
- **Frontend**: Svelte 5 + Vite
- **Notifications**: Discord Webhook

## Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) v22+ (for frontend build)
- A Cloudflare account (Cloudflare Access is used for authentication)

## Setup

### 1. Google Cloud Project (only if using the channel sync button)

If you want to use the "Channel Sync (YouTube)" button in the header menu, you need a GIS client ID.
You can skip this step if you only add channels manually by channel ID.

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project (or select an existing one)
3. Navigate to **APIs & Services > Library** and enable **YouTube Data API v3**
4. Open **Google Auth platform** from the left menu and configure the OAuth consent screen
   - **Audience**: select **External** → add your own Google email address as a test user
   - **Data Access**: add `https://www.googleapis.com/auth/youtube.readonly`
5. Navigate to **APIs & Services > Credentials** from the left menu
6. Click **Create Credentials > OAuth client ID**
   - Application type: **Web application**
   - Authorized JavaScript origins: `http://localhost:3000` (for development)
7. Copy the **Client ID** and set it as `GIS_CLIENT_ID`

> **Note:** No client secret is required. The browser-side GIS (Google Identity Services) obtains a short-lived token and never sends or stores it on the server. Adding yourself as a test user allows usage via the "unverified app" warning screen.

### 2. Configure

Copy `.env.example` to `.env` and fill in your settings:

```env
PORT=3000
DATABASE_PATH=./feed.db
GIS_CLIENT_ID=your-client-id.apps.googleusercontent.com
WEBSUB_CALLBACK_URL=http://localhost:3000/api/websub/callback
PUBLIC_BASE_URL=https://youtube.example.com
```

### 3. Start the Server

```bash
# Development (with frontend hot rebuild)
./bin/dev

# — or —

# Production
cd client && npm install && npx vite build && cd ..
cargo build --release
./target/release/youtube-sub-feed
```

Open `http://localhost:3000`. In development, the first DB user is automatically authenticated (devbypass). In production, Cloudflare Access handles authentication.

### 4. Discord Notifications (Optional)

To receive Discord notifications when new videos are detected:

1. In your Discord server, open **Server Settings > Integrations > Webhooks**
2. Click **New Webhook**, choose a channel, and copy the **Webhook URL**

Add to `.env`:

```env
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/xxx/xxx
```

Restart the server. An embed will be sent for each new video detected via WebSub push.

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

For production, place Cloudflare Access in front of the app. See `docs/deploy.md` for details.

## How It Works

- Channels are registered manually (by channel ID) or bulk-imported via the "Channel Sync (YouTube)" button in the header menu
- On registration, a WebSub (PubSubHubbub) subscription is automatically set up to receive push notifications for new videos
- New video detection runs via WebSub push as the primary mechanism — zero Google API calls required
- Videos can be organized into groups, hidden via swipe, and filtered by type (Shorts, livestreams)

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | Server port |
| `DATABASE_PATH` | `./feed.db` | SQLite database file path |
| `GIS_CLIENT_ID` | — | Google Identity Services client ID (for channel sync button; public value, no secret required) |
| `WEBSUB_CALLBACK_URL` | `http://localhost:3000/api/websub/callback` | WebSub notification endpoint (production requires a public HTTPS URL) |
| `PUBLIC_BASE_URL` | Request origin | Canonical public origin used by feed links (for example, `https://youtube.example.com`) |
| `DISCORD_WEBHOOK_URL` | — | Discord Webhook URL (optional) |

## Commands

| Command | Description |
|---------|-------------|
| `./bin/dev` | Start dev server with frontend hot rebuild |
| `cargo build --release` | Build for production |
| `cargo test` | Run all tests |
