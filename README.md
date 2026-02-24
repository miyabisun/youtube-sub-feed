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
- A Google account with YouTube subscriptions

## Setup

### 1. Google Cloud Project

You need a Google Cloud project with the YouTube Data API enabled and OAuth 2.0 credentials configured.

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project (or select an existing one)
3. Navigate to **APIs & Services > Library**
4. Search for **YouTube Data API v3** and click **Enable**
5. Open **Google Auth platform** from the left menu and configure the OAuth consent screen
   1. **Branding**: enter an app name (e.g. "youtube-sub-feed") and support email
   2. **Audience**: select **External** → add your own Google email address as a test user
   3. **Data Access**: click **Add or remove scopes** and add `https://www.googleapis.com/auth/youtube.readonly`
6. Navigate to **APIs & Services > Credentials** from the left menu
7. Click **Create Credentials > OAuth client ID**
   - Application type: **Web application**
   - Name: anything (e.g. "youtube-sub-feed")
   - Authorized JavaScript origins: (leave empty)
   - Authorized redirect URIs: add `http://localhost:3000/api/auth/callback`
8. Copy the **Client ID** and **Client Secret**

> **Note:** While the app is in "Testing" status on the consent screen, only the test users you added can log in. This is fine for personal use.

### 2. Configure

Copy `.env.example` to `.env` and fill in your credentials:

```env
PORT=3000
DATABASE_PATH=./feed.db
GOOGLE_CLIENT_ID=your-client-id.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET=your-client-secret
GOOGLE_REDIRECT_URI=http://localhost:3000/api/auth/callback
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

Open `http://localhost:3000`, click "Google Login", and authorize. The app will automatically sync your subscriptions and start fetching videos.

### 4. Discord Notifications (Optional)

To receive Discord notifications when new videos are detected:

1. In your Discord server, open **Server Settings > Integrations > Webhooks**
2. Click **New Webhook**, choose a channel, and copy the **Webhook URL**

Add to `.env`:

```env
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/xxx/xxx
```

Restart the server. An embed will be sent for each new video detected during polling.

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

For production, update `GOOGLE_REDIRECT_URI` in `.env` to match your actual domain (e.g. `https://feed.example.com/api/auth/callback`), and add the same URI to the authorized redirect URIs in Google Cloud Console.

## How It Works

- On first login, the app syncs all your YouTube subscriptions and fetches recent videos
- Two polling loops run in the background:
  - **New video detection** (15 min cycle): RSS-First strategy for all `show_livestreams=0` channels — only calls the YouTube API when RSS detects new videos
  - **Livestream detection** (5 min cycle): API-direct polling for `show_livestreams=1` channels, with livestream end detection
- Subscription list syncs every 10 minutes
- Videos can be organized into groups, hidden via swipe, and filtered by type (Shorts, livestreams)

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | Server port |
| `DATABASE_PATH` | `./feed.db` | SQLite database file path |
| `GOOGLE_CLIENT_ID` | — | Google OAuth client ID (required) |
| `GOOGLE_CLIENT_SECRET` | — | Google OAuth client secret (required) |
| `GOOGLE_REDIRECT_URI` | `http://localhost:3000/api/auth/callback` | OAuth callback URL |
| `DISCORD_WEBHOOK_URL` | — | Discord Webhook URL (optional) |

## Commands

| Command | Description |
|---------|-------------|
| `./bin/dev` | Start dev server with frontend hot rebuild |
| `cargo build --release` | Build for production |
| `cargo test` | Run all tests |
