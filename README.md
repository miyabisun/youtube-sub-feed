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
NODE_ENV=production ./target/release/youtube-sub-feed
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
  -e NODE_ENV=production \
  youtube-sub-feed
```

For production, place Cloudflare Access in front of the app. See `docs/deploy.md` for details.

## How It Works

- Channels are registered manually (by channel ID) or bulk-imported via the "Channel Sync (YouTube)" button in the header menu
- On registration, a WebSub (PubSubHubbub) subscription is automatically set up to receive push notifications for new videos
- New video detection runs via WebSub push as the primary mechanism — zero Google API calls required
- Videos can be organized into groups, hidden via swipe, and filtered by type (Shorts, livestreams)

## Environment Variables

The server loads `.env` at startup; existing process environment values take precedence.
All variables can be omitted for startup. Feature-specific requirements and application defaults
are listed below; a release build does not automatically set `NODE_ENV=production`.

| Variable | Required | Unset default | Purpose and invalid/empty values |
| --- | --- | --- | --- |
| `PORT` | No | `3000` | Listen port. Values that cannot parse as `u16` (including empty, negative, or above 65535) fall back to 3000. `0` lets the OS choose a port; bind failure stops startup. |
| `DATABASE_PATH` | No | `./feed.db` | SQLite file. Its parent directory must exist and be writable; failure to open the DB stops startup. Empty is passed directly to SQLite. |
| `GIS_CLIENT_ID` | For browser channel sync unless supplied at build time | Empty (use build-time fallback) | Public Google Identity Services client ID, injected at runtime. Empty falls back to the bundled `VITE_GIS_CLIENT_ID`; sync is unavailable only when both are empty. Nonempty values take precedence without client-ID validation and may fail at Google authorization. |
| `WEBSUB_CALLBACK_URL` | Public HTTPS URL for WebSub delivery | `http://localhost:3000/api/websub/callback` | Callback supplied to the hub. The default does not follow `PORT`. Empty or malformed values are passed through without URL validation and may cause subscription or delivery failures. |
| `PUBLIC_BASE_URL` | No | Request origin | Public origin for feed links, e.g. `https://youtube.example.com`. Surrounding whitespace and trailing `/` are removed; empty falls back to the request. Other values are used without URL validation and can produce malformed links. |
| `DISCORD_WEBHOOK_URL` | For Discord notifications | Disabled | Empty disables notifications; other values are used without trimming or URL validation. Invalid values fail when sending a notification. |
| `YOUTUBE_API_KEY` | For video enrichment and catch-up scans | Disabled | YouTube Data API key. Surrounding whitespace is removed; empty disables those features. Other values are not validated at startup; invalid keys fail at the API. WebSub push can still work without a key. |
| `CATCHUP_INTERVAL_MINUTES` | For periodic catch-up checks | Disabled | Positive integer minutes between `videoCount` checks. Whitespace is trimmed; empty, zero, negative, unparseable, or values exceeding `u64::MAX / 60` disable periodic checks. Requires `YOUTUBE_API_KEY`; startup and manual full sweeps remain available when periodic checks are disabled. |
| `NODE_ENV` | Set `production` for production user handling | Development mode | Exactly `production` disables the first-DB-user fallback for requests without the Access email header and keeps cached SPA HTML. Every other value, including empty or misspelled values, enables development behavior. This does not configure Cloudflare Access itself. |
| `RUST_LOG` | No | `info` | Log level or target filter, e.g. `debug` or `youtube_sub_feed=debug`. Empty selects `error`. Invalid filter syntax prints a warning to stderr and disables logs. `LOG_LEVEL` is not read. |

Without `PUBLIC_BASE_URL`, feed links use `X-Forwarded-Proto` (default `http`),
then `X-Forwarded-Host` or `Host` (default `localhost:<PORT>`).
The listener binds to `0.0.0.0`. Dockerfile sets only `PORT=3000`, not production mode.
Set `NODE_ENV=production` in the server environment (for example, `docker run -e NODE_ENV=production`).
See [deployment instructions](docs/deploy.md) for provider configuration and the
[home-server README](https://github.com/miyabisun/home-server/blob/main/README.md) and
[sis/compose.yaml](https://github.com/miyabisun/home-server/blob/main/sis/compose.yaml)
for shared environment names, mounts, and explicitly injected values. For example,
`YOUTUBE_GIS_CLIENT_ID` and `YOUTUBE_DISCORD_WEBHOOK_URL` are Compose identifiers;
the application reads `GIS_CLIENT_ID` and `DISCORD_WEBHOOK_URL`. The current Compose
file does not set `NODE_ENV`; production mode must be supplied by the deployment environment.

Old server-side OAuth variables `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, and
`GOOGLE_REDIRECT_URI` are no longer read. Browser sync uses `GIS_CLIENT_ID`;
there is no replacement server-side client secret or redirect variable.

### Frontend build setting

`VITE_GIS_CLIENT_ID` is optional and is read by Vite when building the frontend,
not by the running server. It defaults to empty and embeds a public GIS client ID
as the fallback when runtime `GIS_CLIENT_ID` is empty. There is no client-ID
validation; an invalid value may fail at Google authorization. Changing this
fallback requires rebuilding the frontend. A nonempty runtime `GIS_CLIENT_ID`
overrides it without rebuilding.

Sources: [configuration](src/config.rs), [startup](src/main.rs),
[user handling](src/middleware.rs), [SPA cache](src/spa.rs), [feed URLs](src/routes/news.rs),
[frontend configuration](client/src/lib/config.js).

## Commands

| Command | Description |
|---------|-------------|
| `./bin/dev` | Start dev server with frontend hot rebuild |
| `cargo build --release` | Build for production |
| `cargo test` | Run all tests |
