# youtube-sub-feed

A personal web app for browsing your YouTube subscriptions chronologically, without the recommendation algorithm.

## Prerequisites

- [Bun](https://bun.sh/) v1.0+
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

### 2. Install & Configure

```bash
bun run setup
```

This installs dependencies and creates a `.env` file from `.env.example`. Edit `.env`:

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
bun run dev

# — or —

# Production
bun run build
bun start
```

Open `http://localhost:3000`, click "Google Login", and authorize. The app will automatically sync your subscriptions and start fetching videos.

### 4. Discord Notifications (Optional)

To receive Discord notifications when new videos are detected:

1. Go to [Discord Developer Portal](https://discord.com/developers/applications)
2. Create a new application and go to the **Bot** tab
3. Click **Reset Token** and copy the bot token
4. Enable **MESSAGE CONTENT INTENT** under Privileged Gateway Intents (not strictly required, but recommended)
5. Go to **OAuth2 > URL Generator**
   - Scopes: `bot`
   - Bot Permissions: `Send Messages`, `Embed Links`
   - Copy the generated URL and open it to invite the bot to your server
6. In Discord, enable Developer Mode (Settings > Advanced), right-click the channel for notifications, and click **Copy Channel ID**

Add to `.env`:

```env
DISCORD_TOKEN=your-bot-token
DISCORD_CHANNEL_ID=your-channel-id
```

Restart the server. The bot will send an embed for each new video detected during polling.

## Docker

A pre-built image is published to GitHub Container Registry on every push to `main`.

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

Or build locally:

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

For production, update `GOOGLE_REDIRECT_URI` in `.env` to match your actual domain (e.g. `https://feed.example.com/api/auth/callback`), and add the same URI to the authorized redirect URIs in Google Cloud Console.

## How It Works

- On first login, the app syncs all your YouTube subscriptions and fetches recent videos
- Two polling loops run in the background:
  - **Normal** (30 min cycle): rotates through all channels
  - **Fast** (10 min cycle): for channels you mark as "fast polling"
- A daily sync checks for new/removed subscriptions
- Videos can be organized into groups, hidden via swipe, and filtered by type (Shorts, livestreams)

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | Server port |
| `DATABASE_PATH` | `./feed.db` | SQLite database file path |
| `GOOGLE_CLIENT_ID` | — | Google OAuth client ID (required) |
| `GOOGLE_CLIENT_SECRET` | — | Google OAuth client secret (required) |
| `GOOGLE_REDIRECT_URI` | `http://localhost:3000/api/auth/callback` | OAuth callback URL |
| `DISCORD_TOKEN` | — | Discord bot token (optional) |
| `DISCORD_CHANNEL_ID` | — | Discord channel for notifications (optional) |

## Commands

| Command | Description |
|---------|-------------|
| `bun run setup` | Install dependencies and create `.env` |
| `bun run dev` | Start dev server with frontend hot rebuild |
| `bun run build` | Build frontend for production |
| `bun start` | Start production server |
| `bun test` | Run all tests |
