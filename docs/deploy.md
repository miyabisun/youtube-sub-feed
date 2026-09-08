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

タグ `vX.Y.Z` が `Cargo.toml` / `Cargo.lock` の package version と一致する
commit から、Linux amd64 の image を `ghcr.io/miyabisun/youtube-sub-feed:X.Y.Z`
と `:latest` に公開します。GitHub Release や native binary は作成しません。

Rust 1.96.0 と cargo-chef 0.1.78 を固定し、依存 build の後に本物の manifest と
source をコピーして本体を再コンパイルします。release profile は `opt-level=3`、
`lto=false`、`codegen-units=16`、`strip=true` です。frontend は lockfile に
従う `npm ci` を使い、成果物を従来どおり `/app/client/build` に配置します。
CI は製品 image と分けた GHCR の `:build-cache` に中間段階を `mode=max` で保存し、
次のタグで再利用します。cache が無い初回も通常の build で公開できます。

## Docker 起動

```bash
docker run -d \
  --name youtube-sub-feed \
  -p 3000:3000 \
  -v /path/to/data:/data \
  -e NODE_ENV=production \
  -e DATABASE_PATH=/data/feed.db \
  -e GIS_CLIENT_ID=xxx.apps.googleusercontent.com \
  -e YOUTUBE_API_KEY=AIzaXXXX \
  -e CATCHUP_INTERVAL_MINUTES=10 \
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
（再生時間・プレーヤー寸法による Shorts 判定・ライブ配信状態）のエンリッチと、
WebSub の取りこぼし検出に使用します。未設定でも WebSub push は動作しますが、
Shorts フィルタ、ライブ判定、取りこぼし検出は機能しません。

1. [Google Cloud Console](https://console.cloud.google.com/) で「APIとサービス」→「ライブラリ」から **YouTube Data API v3** を有効化
2. 「認証情報」→「認証情報を作成」→「APIキー」を作成
3. 作成したキーを `YOUTUBE_API_KEY` に設定（「APIの制限」で YouTube Data API v3 のみに絞ることを推奨）

## WebSub に依存しない API 巡回

`YOUTUBE_API_KEY` と `CATCHUP_INTERVAL_MINUTES=10` を設定すると、起動時と10分ごとに
API巡回します。Hubの503・長いRetry-After・応答遅延・購読確認成功なのにpushが無い場合も
同じ処理を継続します。WebSub購読更新は別workerで動き、APIの排他を保持しません。

1巡回の上限は、count未設定・増減を検出した先頭 **4チャンネル**、24時間経過した先頭の
修復 **2チャンネル**、履歴cursorの継続 **2チャンネル**、それぞれ1ページ（最大50件）です。
同じ先頭ページを複数の枠で取りません。先頭が毎回変わるチャンネルも、別ページの履歴cursorは進めます。
先頭の修復は履歴の深さから独立し、同数入替・統計の反映遅延・統計からの欠落も後から照合します。
履歴はAPIで公開取得できるuploads全体が対象で、一周完了から7日後に次の一周を始めます。
APIのpage tokenはスナップショットではないため、途中の削除・並び替えによる飛びは次の照合で修復します。
無効なtokenは当該履歴だけ先頭へ戻し、他チャンネルは継続します。

`channel_catchup` の試行時刻順で処理し、失敗チャンネルも後ろへ回します。各ページの動画、
videoCount、次のcursorは同じtransactionで保存します。どれかの保存失敗で全て巻き戻り、
次回の再試行対象を失いません。再起動は保存済みcursorと修復期限を引き継ぎます。
既存DBには初回起動時に進捗テーブルと `videos.details_attempted_at` を追加します。

新規動画の詳細はチャンネルをまたいで最大50 IDずつ取得します。未完了の詳細も1巡回で100 IDまで
補完し、試行時刻で分散、失敗からの再試行は10分以上空けます。ライブの再確認は成功から24時間後です。
push・poll・補完は同じ保存処理と詳細の排他を使い、既に補完済みのIDを再取得しません。

手動の「全件取得し直し」は全チャンネルの先頭を修復対象へ戻し、まず上限付きの1巡回を実行します。
残りと履歴は定期巡回で続けます。`CATCHUP_INTERVAL_MINUTES` が未設定・無効なら起動/手動の
1巡回だけで止まり、残りを継続するには定期巡回を有効にするか手動で再実行する必要があります。
HTTP 202や初回巡回の完了通知は、履歴全部の回収完了を意味しません。

### 費用と取得遅延

[channels.list](https://developers.google.com/youtube/v3/docs/channels/list)、
[playlistItems.list](https://developers.google.com/youtube/v3/docs/playlistItems/list)、
[videos.list](https://developers.google.com/youtube/v3/docs/videos/list) は各1 unit/requestです。
以下は165チャンネル・10分間隔・正常応答を前提とした**この実装の試算**で、実projectのquota値ではありません。
「/日」は24時間・144巡回の換算です。PTの25時間の日は150巡回となり、統計600、全retryを含む
保守上限9900 unitsに起動・手動・push・他用途分を加算します。

| 状態 | API要求数 / 推計units | 取得遅延の目安・上限 |
| --- | --- | --- |
| 変更・修復・詳細待ちなし | 統計 `ceil(165/50)=4` / 巡回、`4×144=576` / 日 | count差分は通常次の10分tick |
| 通常運用・WebSub長期停止 | 上記 + 変更先頭のページ数 + 先頭修復（正常時最大165/日）+ 履歴ページ数（最大288/日）+ 詳細batch数 + retry | 165chが同時に変化すると先頭は最大 `ceil(165/4)×10分=7時間` |
| count相殺・反映遅延・統計欠落 | 24h経過後に2ch/tickで先頭を再照合。1巡回の修復追加は最大2 | 先頭50件は保守的に `24h+ceil(165/2)×10分=37時間50分` |
| 初回/履歴バックログ | 全枠を使う極端な巡回は統計4 + playlist8 + 新規詳細8 + 既存詳細2 = **22 / 巡回、3168 / 日** | 1chの履歴をPページとすると保守的に `ceil(165/2)×P×10分`。2回目以降の履歴修復には7日を加算 |
| retryも含む保守上限 | 各要求最大3試行なので **66 / 巡回、9504 / 日** | HTTP試行は各10秒、通常backoffは2秒・4秒。長いRetry-Afterやquota休止は別途加算 |

起動時の1巡回、手動実行、push時の詳細取得、ブラウザOAuth同期、同じGoogle projectの他用途は
日次試算に別途加算します。ブラウザの `subscriptions.list` は最大50件ずつページングし、
サーバーの利用数記録には入りません。既に成功した詳細を再取得しないため、通常時の費用は上表の
極端な上限より小さくなります。失敗・休止期間とYouTube側の未公開/未反映時間は遅延に加算してください。
継続的な大量公開、アクセス不能、変化し続けるページ順について無条件の有限遅延は保証しません。

### 実project予算との照合と休止

運用前にGoogle Cloud Consoleで **APIキーのprojectの実際の日次quota、当日の利用量、
GIS_CLIENT_IDや他アプリと共有する用途**を確認します。一般の既定値を実機の上限として扱いません。
[公式quota説明](https://developers.google.com/youtube/v3/determine_quota_cost) にも、無効な要求の
課金単位とPacific Timeの午前0時リセットが記載されています。

`YOUTUBE_API_DAILY_BUDGET` は、このサーバーへ割り当てた要求数を制限する任意の非負整数です。
未設定/空はローカル上限なし、0はAPI要求停止、不正な整数は起動エラーです。値を決める際は、
統計576だけでなく上表の修復・詳細・retry・push・手動分を含めてください。

この制限は**UTC午前0時で区切る日次窓**です。GoogleのPT日次窓とは異なり、PT日は
UTCの2日分にまたがります。DSTの25時間の日も含め、PT日ごとの確実な予約が必要なら
`2×設定値 + ブラウザ/他用途 + 予備 <= 実project上限` を満たす割当てにします。
必要量と実予算が両立しなければ、間隔や同一projectの用途を見直してください。

要求直前に `youtube_api_state` へ試行数を保存し、失敗/retryも1試行1 unitで見積もります。
上限到達は窓の終了まで、APIの `quotaExceeded` / `dailyLimitExceeded` は検出から25時間、
キーを使う全経路を休止します。25時間はDSTの日も次のPT午前0時を越える保守的な待機です。
再起動で残予算や休止を解除しません。期限後の次の巡回/要求が再開し、quotaが続けば再休止します。
403のrateLimitは日次quotaと区別してretryします。30秒を超すRetry-Afterは秒数/HTTP日時を
共有の休止期限として保存し、workerを長時間sleepさせません。短い指定も次の試行まで待ちます。

WebSubの診断・購読・解除は従来の10秒以上のpacingを使い、Retry-Afterは最後の失敗後の
次チャンネルにも適用します。WebSub失敗通知はbatchで集約し、API失敗通知は理由ごとに1時間に1回まで。
少数のpush成功やHub受付だけでAPIを止めません。

### リリース後の確認（home-server/sis）

以下は本番接続元でのread-only確認です。実装テストはlocalhost stubと一時SQLiteだけを使います。
imageのrevisionを今回のrelease commitと照合し、起動直後から少なくとも35分（複数巡回）を観測します。

```bash
docker --context conoha inspect --format '{{.Image}} {{.State.StartedAt}} {{.RestartCount}}' sis-youtube-1
image_id=$(docker --context conoha inspect --format '{{.Image}}' sis-youtube-1)
docker --context conoha image inspect --format '{{index .Config.Labels "org.opencontainers.image.version"}} {{index .Config.Labels "org.opencontainers.image.revision"}}' "$image_id"
docker --context conoha logs --since 35m sis-youtube-1 2>&1 | rg '\[catchup\] (Scan started|Scan complete|page saved)|\[youtube-api\] (quota pause|Retry-After pause)'
```

推計unitsは `[youtube-api] request` をendpoint別に数えます。APIキーやURL、環境変数全体を出力しません。

```bash
docker --context conoha logs --since 24h sis-youtube-1 2>&1 | python3 -c '
import collections, json, re, sys
counts = collections.Counter()
for line in sys.stdin:
    line = re.sub(r"\x1b\[[0-9;]*m", "", line)
    if "[youtube-api] request" in line:
        match = re.search(r"endpoint=\"?(\w+)", line)
        if match:
            counts[match[1]] += 1
print(json.dumps({"requests": dict(counts), "estimated_units": sum(counts.values())}))
'
docker --context conoha exec sis-sqlite-backup-1 sqlite3 -readonly /dbs/youtube/feed.db 'SELECT requests, datetime(window_started,"unixepoch"), datetime(quota_until,"unixepoch"), datetime(retry_until,"unixepoch") FROM youtube_api_state; SELECT count(*) AS pending_heads FROM channel_catchup WHERE repair_after <= unixepoch(); SELECT count(*) AS active_cursors FROM channel_catchup WHERE page_token IS NOT NULL; SELECT count(*) AS unchecked_details FROM videos WHERE details_checked_at IS NULL;'
```

Google Consoleの日次使用量とも照合します。ログはコンテナ再作成で失われ、DBの`requests`はUTC日次窓で
リセットされます。いずれも他アプリ/ブラウザの要求は含まず、実project全体の残quotaではありません。
`page saved imported=N` が実際の新規登録数です。`fetched_at`は既存行の照合でも更新するため、
その件数だけで新着の回収成功とはしません。

最後に通常の認証済みブラウザで `/api/feed` と画面を確認し、APIで取り込んだ動画IDが表示されることを
照合します。内部Docker networkからAPIを確認する場合の例（既存backupコンテナに`wget`がある場合）:

```bash
docker --context conoha exec sis-sqlite-backup-1 sh -eu -c '
email=$(sqlite3 -readonly /dbs/youtube/feed.db "SELECT email FROM users ORDER BY id LIMIT 1")
test -n "$email"
wget -qO- --header="Cf-Access-Authenticated-User-Email: $email" http://youtube:3000/api/feed
' | python3 -c 'import json, sys; rows=json.load(sys.stdin); assert isinstance(rows, list); print({"feed_count": len(rows), "video_ids": [v["id"] for v in rows[:10]]})'
```

このコマンドはemailや認証情報を表示しません。内部経路だけの成功はCloudflare経由の画面確認を代替しません。
Hub障害の残存や全チャンネル購読成功を待たず、API巡回継続・取り込み・通常feed・費用の4点で確認します。

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

一覧と不正値の扱いは [README.md](../README.md#environment-variables) を正本とします。
本番では `NODE_ENV=production` を明示してください。Dockerfile や release build は
この値を設定しません。上記起動例では `-e NODE_ENV=production` で渡します。

## 削除された環境変数（旧 Google OAuth）

以下の環境変数は不要になりました。旧バージョンから移行する場合は削除してください:

- `GOOGLE_CLIENT_ID` — Google OAuth2 クライアントID（不要）
- `GOOGLE_CLIENT_SECRET` — Google OAuth2 クライアントシークレット（不要）
- `GOOGLE_REDIRECT_URI` — OAuth2 コールバックURL（不要）
