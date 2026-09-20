# デプロイ手順

## 前提: Cloudflare Access の設定

[Cloudflareの公開アプリ設定](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/self-hosted-public-app/)に従い、
自分のドメインからアプリへTunnelで転送し、許可するメールアドレスをAccessのポリシーへ指定します。

このアプリは認証に **Cloudflare Access** を使用します。
Cloudflare Zero Trust でアプリケーションを作成し、アクセスポリシーを設定してください。
Cloudflare Access は認証済みリクエストに `Cf-Access-Authenticated-User-Email` ヘッダーを付与します。
このヘッダーをサーバーが読み取り、ユーザーを特定します。

> **セキュリティ重要**: アプリの **3000 番ポートを外部から直接到達可能な状態にしないこと**。
> 必ず Cloudflare Tunnel / Cloudflare Access 経由でのみアクセスできるように設定してください。
> `Cf-Access-Authenticated-User-Email` ヘッダーはアプリ側で無検証で信頼するため、
> ポートが直接公開されるとヘッダー偽装で任意ユーザーになりすませます。

## WebSub callback

`/api/websub/callback`の正確なパスだけに、AccessのBypassポリシーを設定します。
Google Hubは対話ログインできないため、このパスのGETによる購読確認とPOST通知を通す必要があります。
アプリは保存した購読secretでPOSTの署名を検証します。
画面や他のAPIまで認証を除外せず、callbackの公開とoriginポートの公開を区別してください。
詳細は[Accessのパス設定](https://developers.cloudflare.com/cloudflare-one/access-controls/policies/app-paths/)と
[YouTubeのpush通知](https://developers.google.com/youtube/v3/guides/push_notifications)を参照してください。

## Docker ビルド

ソースを取得したcheckoutのルートで実行します。公開イメージからの起動は[README](../README.md#run-with-docker)を参照してください。

```bash
docker build -t youtube-sub-feed .
```

タグ`vX.Y.Z`は`Cargo.toml`と`Cargo.lock`のpackage versionに一致させます。
そのcommitからLinux amd64のimageを公開します。
公開先は`ghcr.io/miyabisun/youtube-sub-feed:X.Y.Z`と`:latest`です。GitHub Release や native binary は作成しません。

Rust 1.96.0 と cargo-chef 0.1.78 を固定し、依存 build の後に本物の manifest と
source をコピーして本体を再コンパイルします。release profile は `opt-level=3`、
`lto=false`、`codegen-units=16`、`strip=true` です。frontend は lockfile に
従う `npm ci` を使い、成果物を従来どおり `/app/client/build` に配置します。
CI は製品 image と分けた GHCR の `:build-cache` に中間段階を `mode=max` で保存し、
次のタグで再利用します。cache が無い初回も通常の build で公開できます。

## Docker 起動

データ用ディレクトリを事前に作り、`/path/to/data`を置き換えます。
APIキーとDiscord URLを使う場合は、実行するシェルに同名の環境変数を設定してください。

```bash
docker run -d \
  --name youtube-sub-feed \
  -p 127.0.0.1:3000:3000 \
  -v /path/to/data:/data \
  -e NODE_ENV=production \
  -e DATABASE_PATH=/data/feed.db \
  -e GIS_CLIENT_ID=xxx.apps.googleusercontent.com \
  -e YOUTUBE_API_KEY \
  -e CATCHUP_INTERVAL_MINUTES=10 \
  -e WEBSUB_CALLBACK_URL=https://youtube.example.com/api/websub/callback \
  -e PUBLIC_BASE_URL=https://youtube.example.com \
  -e DISCORD_WEBHOOK_URL \
  youtube-sub-feed
```

WebSubで新着動画を受信するため、`WEBSUB_CALLBACK_URL`へ公開HTTPS URLを指定してください。

## GIS_CLIENT_ID の設定

ブラウザの「チャンネル同期 (YouTube)」を使う場合だけ設定します。
手動でチャンネルIDを登録するなら不要です。
[GoogleのクライアントID設定](https://developers.google.com/identity/oauth2/web/guides/get-google-api-clientid)も参照してください。

1. [Google Cloud Console](https://console.cloud.google.com/) にアクセス
2. 新しいプロジェクトを作成（または既存のプロジェクトを選択）
3. **API とサービス > ライブラリ** に移動し、**YouTube Data API v3** を有効にする
4. 左メニューの **Google Auth platform** を開き、OAuth 同意画面を設定
   - 対象: **外部** を選択 → テストユーザーに自分の Google メールアドレスを追加
   - データアクセス: `https://www.googleapis.com/auth/youtube.readonly` を追加
5. 左メニューの **API とサービス > 認証情報** に移動
6. **認証情報を作成 > OAuth クライアント ID** をクリック
   - アプリケーションの種類: **ウェブ アプリケーション**
   - 承認済みの JavaScript 生成元: `https://youtube.example.com`（自分の公開originへ置換）
7. 作成した **クライアント ID** を `GIS_CLIENT_ID` に設定する

クライアントシークレットは不要です。ブラウザのGISが短命トークンを取得します。
このサーバーへトークンを送信・保存しません。テストユーザーに自分を追加すれば「未確認アプリ」警告を経由して利用できます。

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

1巡回では、count未設定・増減を検出した先頭4チャンネルを確認します。
さらに24時間経過した先頭の修復2チャンネル、履歴cursorの継続2チャンネルを処理します。
それぞれ1ページ（最大50件）が上限です。
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

手動の「全件取得し直し」は全チャンネルの先頭を修復対象へ戻し、まず上限付きで1巡回します。
残りと履歴は定期巡回で続けます。`CATCHUP_INTERVAL_MINUTES` が未設定・無効なら起動/手動の
1巡回だけで止まり、残りを継続するには定期巡回を有効にするか、手動で再実行してください。
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

運用前にGoogle Cloud Consoleで、APIキーのprojectの日次quotaと当日の利用量を確認します。
GIS_CLIENT_IDや他アプリと共有する用途も照合してください。一般の既定値を実機の上限として扱いません。
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
次チャンネルにも適用します。WebSub失敗通知はbatchで集約し、API失敗通知は理由ごとに、1時間あたり1回までです。
少数のpush成功やHub受付だけでAPIを止めません。

### 配備後の確認

以下は起動したホスト上での確認例です。コンテナ名は起動時に指定した名前へ合わせます。

```bash
docker inspect --format '{{.Image}} {{.State.StartedAt}} {{.RestartCount}}' youtube-sub-feed
image_id=$(docker inspect --format '{{.Image}}' youtube-sub-feed)
docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.version"}} {{index .Config.Labels "org.opencontainers.image.revision"}}' "$image_id"
docker logs --since 35m youtube-sub-feed 2>&1 | rg '\[catchup\] (Scan started|Scan complete|page saved)|\[youtube-api\] (quota pause|Retry-After pause)'
```

公開imageのrevisionを対象tagのcommitと照合します。
10分間隔なら少なくとも35分観測し、複数巡回の継続と`page saved imported=N`の新規登録数を確認します。
`fetched_at`は既存行の照合でも更新するため、その件数だけで回収成功とはしません。
Google Cloud Consoleで同じprojectの日次使用量と残quotaも確認してください。
アプリのログやUTC日次予算だけでは、ブラウザや他アプリも含むproject全体の利用量は分かりません。
ログはコンテナ再作成で失われます。APIキーやURL、環境変数全体を診断出力に含めないでください。

認証済みブラウザからフィードを開き、取り込んだ動画が表示されることを確認します。
内部経路のAPI応答だけではCloudflare経由の利用確認になりません。
Hubの全購読成功だけに依存せず、API巡回・取り込み・通常のフィード・API予算を照合します。

## 初回セットアップ

1. コンテナ起動後、Cloudflare Access 経由で最初にアクセスしたユーザーが **マスターユーザー** として自動登録されます。
2. ヘッダーメニューの「チャンネル同期 (YouTube)」でGoogleアカウントを認可し、購読一覧を取り込みます。
   手動追加は「チャンネル」でUCから始まるチャンネルIDを入力します。
3. チャンネルを追加すると WebSub サブスクリプションが自動的に登録され、新着動画がプッシュ通知されます。

## ローカル開発

本番と別のDBを使い、開発サーバーへ他の端末から到達できない環境で実行します。
listenerは`0.0.0.0`なので、開発モードを公開ネットワークへ露出しないでください。
`.env`で開発用DBを選び、`NODE_ENV`を`development`にして、READMEの手順で画面をビルドします。
`./bin/dev`で起動した後、空のDBへローカル検証用ユーザーを作る場合だけ次を実行します。

```bash
curl --fail http://localhost:3000/api/auth/me \
  -H 'Cf-Access-Authenticated-User-Email: dev@example.com'
```

ブラウザで`http://localhost:3000`を開くと、このDBの最初のユーザーを使います。
これは自分の隔離環境での初期化です。本番はヘッダーを手動で作らず、Accessによる認証を使ってください。

## Discord通知

Discordのサーバー設定から連携サービスのWebhookを作り、通知先チャンネルを選びます。
コピーしたURLを`DISCORD_WEBHOOK_URL`に設定してアプリを再起動してください。
WebSubの不正なpush、購読・API取得の失敗通知に使います。URLは秘密値として扱ってください。

## Volume マウント

- `/data/feed.db` — SQLite データベースファイル
- コンテナ再起動時もデータが永続化される

## 環境変数

環境変数の一覧と不正値の扱いは、[README.md](../README.md#environment-variables) を参照してください。
本番では `NODE_ENV=production` を明示してください。Dockerfile や release build は
この値を設定しません。上記起動例では `-e NODE_ENV=production` で渡します。

## 削除された環境変数（旧 Google OAuth）

以下の環境変数は不要になりました。旧バージョンから移行する場合は削除してください:

- `GOOGLE_CLIENT_ID` — Google OAuth2 クライアントID（不要）
- `GOOGLE_CLIENT_SECRET` — Google OAuth2 クライアントシークレット（不要）
- `GOOGLE_REDIRECT_URI` — OAuth2 コールバックURL（不要）

## WebSub の購読確認と再試行

起動・チャンネル追加・24時間ごとの更新・手動全件取得では、申請前に購読状態を照会します。
各topicとcallbackをGoogle Hubの[公式Subscriber Diagnostics](https://pubsubhubbub.appspot.com/subscribe)で確認します。
全購読一覧の JSON API ではなく、`GET /subscription-details` の HTML 診断です。
既存の署名 secret を使い、購読が有効で期限まで2日を超えるものは再申請しません。
診断のHTTPエラー・未知のHTML・状態・日時はログに残し、コールバックで確認済みのDB期限へフォールバックします。
その場合、Hubの現在状態を直接確認できたとは扱いません。診断結果でDBの署名secretや確認済み期限を書き換えることもありません。

全経路で同じ送信制御を使い、診断・購読・解除を10秒以上空けて送ります。
一時エラー（通信失敗、408、429、500、502、503、504）は30秒以上空けて2回まで再試行します。
`Retry-After`の秒数・HTTP日時がさらに先なら、その時刻まで待ちます。恒久エラーは再試行しません。
全対象の処理後、最終失敗だけをチャンネル名でDiscordへ1回通知します。
名前が未入力・ID代用の場合だけ公開Atomフィードからチャンネル名を取得して保存し、取得不能時は「名前未取得のチャンネル」と表示します（IDはログに残します）。
起動直後の同じ失敗を定期処理で繰り返さないよう、次の購読更新は24時間後です。動画情報の補完はHubから独立したAPI巡回で行います。
HTTP受付は非同期確認の完了を意味せず、受付後1時間は同じチャンネルの再申請を待ちます。
手動全件取得は全チャンネルをAPIの修復対象に戻し、上限付きの巡回で継続します。
費用・最大遅延・休止と復旧・本番確認は [デプロイ手順](#websub-に依存しない-api-巡回) を参照してください。
[YouTube の公式プッシュ通知手順](https://developers.google.com/youtube/v3/guides/push_notifications) も参照してください。
