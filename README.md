# youtube-sub-feed

[日本語](README.ja.md)

A self-hosted web app for viewing YouTube subscriptions in chronological order.
Add channels manually or import your subscriptions.
Organize videos into groups, hide watched items, and filter Shorts or livestreams.

## Run with Docker

You need Docker on a Linux amd64 host and a [Tailscale](https://tailscale.com/) tailnet.
Users open the app through [Tailscale Serve](https://tailscale.com/kb/1312/serve), which adds the `Tailscale-User-Login` header for a tailnet user's device and strips it from client requests.
The app trusts that header; it does not verify it itself.
Keep the origin port private so every request goes through Serve.
The WebSub callback is the only path that must be reachable from the public internet.
App-specific setup is in the [deployment guide (Japanese)](docs/deploy.md).

Create a `.env` file. Replace the example origin with your domain:

```env
NODE_ENV=production
DATABASE_PATH=/data/feed.db
PUBLIC_BASE_URL=https://youtube.example.com
WEBSUB_CALLBACK_URL=https://youtube.example.com/api/websub/callback
```

```bash
docker pull ghcr.io/miyabisun/youtube-sub-feed:latest
docker run -d \
  --name youtube-sub-feed \
  -p 127.0.0.1:3000:3000 \
  -v ytfeed-data:/data \
  --env-file .env \
  ghcr.io/miyabisun/youtube-sub-feed:latest
```

Expose it on the tailnet with `tailscale serve --bg 3000` on this host.
To run Tailscale in a container instead, put it on the same private Docker network as the app, point Serve at `http://<container>:3000`, and publish no host port.
Route public GET/POST requests to the exact WebSub callback path only; see [callback setup](docs/deploy.md#websub-callback).

Open the Serve URL (`https://<host>.<tailnet>.ts.net`) from a device signed in to your tailnet.
The first authenticated user becomes the master user. An empty database has no default account.
Setting `NODE_ENV=production` is required even with a release image.

## Add channels and read videos

1. Open **チャンネル** (Channels) and add a channel ID beginning with `UC`.
   Alternatively, configure `GIS_CLIENT_ID` and use **チャンネル同期 (YouTube)** in the header menu.
2. Return to the feed to see imported videos ordered by publication time.
3. Use groups and the Shorts/live filters to narrow the feed; swipe a video to hide it.

Sync matches your channel list to your Google subscriptions, including removals.
Check the Google account before syncing.

WebSub receives new uploads through the public callback. Set `YOUTUBE_API_KEY` for API catch-up and video details.
For periodic checks, set `CATCHUP_INTERVAL_MINUTES` to positive minutes, such as `10`.
Without the key, API catch-up and detail-based classification are unavailable.
Without periodic catch-up, startup/manual refresh runs only one bounded pass.
A new channel may show no videos until push delivery or API retrieval.

## Refresh the feed

The feed does not reload on its own: not on a timer, and not when you come back to its tab or app.
On a touch screen, pull the feed down from its top and let go. A strip above the list shows whether letting go will refresh, and stays while the newest items load.
Switching the group, or the **動画** (Videos) and **投稿** (Posts) toggles, also loads the list again.
Without a touch screen, reload the page.

## Open a video

A video card is a link: its thumbnail and title open the video on YouTube in a new tab, Shorts at their Shorts address.
The list stays where it was, and a video opened from the feed is marked as watched.

Inside [subfeed-browser](https://github.com/miyabisun/subfeed-browser), an Android browser built for this feed, a tap hands the video to the app instead, which opens it on YouTube's own page.
The header carries `data-video-host-keep`. It marks the part a host app is to keep on screen above the video, closing the video when that part is pressed; other browsers ignore it.

## Read channel posts

The feed also shows the public posts from your channels' Posts tab, mixed with videos by time.
Use **動画** (Videos) and **投稿** (Posts) above the feed to choose what appears; both start on.
**返信** (Reply) opens the post on YouTube, where you reply; on a phone, the YouTube app may open it.
To leave one channel's posts out, turn on **投稿NG** (No posts) on that channel's page; its videos still appear.

The app reads the first page of each Posts tab about once an hour, starting at launch, without signing in.
YouTube offers no official API for posts, so this uses its unofficial web endpoint.
A change on YouTube's side can break it; failures are logged and reported to Discord when configured.
Post times are estimated from labels such as "3 days ago". Members-only posts are not shown.

For sync, [create a Google web client ID](https://developers.google.com/identity/oauth2/web/guides/get-google-api-clientid).
Use your app’s public origin. Allow the `https://www.googleapis.com/auth/youtube.readonly` scope.
Set the resulting ID as `GIS_CLIENT_ID`; enable YouTube Data API v3 in the Google project.
API catch-up uses a separate API key from that project, set as `YOUTUBE_API_KEY`.
See the [deployment guide (Japanese)](docs/deploy.md) for quota planning and Discord alerts. Browser sync uses a public GIS client ID, with no client secret.
It does not send its OAuth token to this server.

## Data and updates

The named volume stores the SQLite database. Keep it when replacing the container.
Back up with SQLite's backup facility, or stop the app and copy the entire data directory.
After pulling an updated image, stop and remove only the container.
Then run the same command again.
Keep the same volume and environment file. Do not publish `.env` or its secrets.

## Build from source

The backend uses Rust and SQLite; the frontend uses Svelte 5 and Vite.
Install Git, Rust 1.96+, Node.js 22+ and npm.
You also need pkg-config and OpenSSL development libraries.
Then run:

```bash
git clone https://github.com/miyabisun/youtube-sub-feed.git
cd youtube-sub-feed
cp .env.example .env
npm --prefix client ci
npm --prefix client run build
cargo build --release --locked
NODE_ENV=production ./target/release/youtube-sub-feed
```

Edit `.env` before starting; the default database path is `./feed.db`.
Production uses the same Tailscale Serve setup as Docker. For local development, `./bin/dev` watches the frontend and starts the Rust server.
Development mode reuses the first existing DB user; it does not create one automatically.
See [local development](docs/deploy.md#ローカル開発) for initial setup and isolation.
After building the frontend, check the backend with `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings` and `cargo test --locked`.

## Environment Variables

The server loads `.env` at startup; existing process environment values take precedence.
All variables can be omitted for startup. Feature requirements and defaults are listed below.
A release build does not automatically set `NODE_ENV=production`.

| Variable | Required | Unset default | Purpose and invalid/empty values |
| --- | --- | --- | --- |
| `PORT` | No | `3000` | Listen port. Values that cannot parse as `u16` (including empty, negative, or above 65535) fall back to 3000. `0` lets the OS choose a port; bind failure stops startup. |
| `DATABASE_PATH` | No | `./feed.db` | SQLite file. Its parent directory must exist and be writable; failure to open the DB stops startup. Empty is passed directly to SQLite. |
| `GIS_CLIENT_ID` | For browser channel sync unless supplied at build time | Empty (use build-time fallback) | Public Google Identity Services client ID, injected at runtime. Empty falls back to the bundled `VITE_GIS_CLIENT_ID`; sync is unavailable only when both are empty. Nonempty values take precedence without client-ID validation and may fail at Google authorization. |
| `WEBSUB_CALLBACK_URL` | Public HTTPS URL for WebSub delivery | `http://localhost:3000/api/websub/callback` | Callback supplied to the hub. The default does not follow `PORT`. Empty or malformed values are passed through without URL validation and may cause subscription or delivery failures. |
| `PUBLIC_BASE_URL` | No | Request origin | Public origin for feed links, e.g. `https://youtube.example.com`. Surrounding whitespace and trailing `/` are removed; empty falls back to the request. Other values are used without URL validation and can produce malformed links. |
| `DISCORD_WEBHOOK_URL` | For Discord notifications | Disabled | Empty disables notifications; other values are used without trimming or URL validation. Invalid values fail when sending a notification. |
| `YOUTUBE_API_KEY` | For video enrichment and catch-up scans | Disabled | YouTube Data API key. Surrounding whitespace is removed; empty disables those features. Other values are not validated at startup; invalid keys fail at the API. WebSub push can still work without a key. |
| `YOUTUBE_API_DAILY_BUDGET` | No | No local cap | This server's request budget in a persistent UTC-day window, not the Google project's quota. Empty disables the cap; 0 stops API requests; an invalid nonnegative integer fails startup. Allocate against the actual project quota and other uses; see [quota operation](docs/deploy.md#実project予算との照合と休止). |
| `CATCHUP_INTERVAL_MINUTES` | For periodic catch-up checks | Disabled | Positive integer minutes between `videoCount` checks. Whitespace is trimmed; empty, zero, negative, unparseable, or values exceeding `u64::MAX / 60` disable periodic checks. Requires `YOUTUBE_API_KEY`; startup/manual each run one bounded pass when periodic checks are disabled. Enable periodic checks to drain the remaining work. |
| `NODE_ENV` | Set `production` for production user handling | Development mode | Exactly `production` disables the first-DB-user fallback for requests without the `Tailscale-User-Login` header and keeps cached SPA HTML. Every other value, including empty or misspelled values, enables development behavior. This does not configure Tailscale Serve itself. |
| `RUST_LOG` | No | `info` | Log level or target filter, e.g. `debug` or `youtube_sub_feed=debug`. Empty selects `error`. Invalid filter syntax prints a warning to stderr and disables logs. `LOG_LEVEL` is not read. |

Without `PUBLIC_BASE_URL`, feed links take the protocol from `X-Forwarded-Proto`.
It defaults to `http`. The host comes from `X-Forwarded-Host`, then `Host`.
The host fallback is `localhost:<PORT>`.
The listener binds to `0.0.0.0`. Dockerfile sets only `PORT=3000`, not production mode.
Set `NODE_ENV=production` in the server environment, as in the Docker example above.
See [deployment instructions](docs/deploy.md) for authentication, public callbacks and quota operation.

These old server-side OAuth variables are no longer read:

- `GOOGLE_CLIENT_ID`
- `GOOGLE_CLIENT_SECRET`
- `GOOGLE_REDIRECT_URI`
 Browser sync uses `GIS_CLIENT_ID`.
There is no replacement server-side client secret or redirect variable.

### Frontend build setting

Vite reads the optional `VITE_GIS_CLIENT_ID` at build time.
The running server does not read it. It defaults to empty. Its bundled public ID is used when runtime `GIS_CLIENT_ID` is empty. There is no client-ID
validation; an invalid value may fail at Google authorization. Changing this
fallback requires rebuilding the frontend. A nonempty runtime `GIS_CLIENT_ID`
overrides it without rebuilding.

Sources:

- [Configuration](src/config.rs) and [startup](src/main.rs).
- [User handling](src/middleware.rs) and [SPA cache](src/spa.rs).
- [Feed URLs](src/routes/news.rs) and [frontend configuration](client/src/lib/config.js).
