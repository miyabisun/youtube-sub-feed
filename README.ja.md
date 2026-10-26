# youtube-sub-feed

[English](README.md)

YouTubeの登録チャンネルの動画を、公開日時順に閲覧するWebアプリです。
チャンネルの手動追加や購読一覧の取込み、グループ分け、動画の非表示、ショート・ライブの絞込みに対応します。

## Dockerで起動する

Linux amd64で動くDockerと、[Tailscale](https://tailscale.com/)のtailnetが必要です。
利用者は[Tailscale Serve](https://tailscale.com/kb/1312/serve)を通して開きます。Serveはtailnetの利用者の端末からの要求に`Tailscale-User-Login`ヘッダーを付け、クライアントが送った同名のヘッダーを取り除きます。
アプリはこのヘッダーを信頼し、その正当性を自分では検証しません。
サーバーのポートへServeを経ずに到達できないようにしてください。
公開インターネットから届く必要があるのはWebSubのcallbackだけです。
[デプロイ手順](docs/deploy.md)に従い、この経路とアクセスポリシーを設定します。

次の内容で`.env`を作り、公開URLを自分のドメインへ置き換えます。

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

このホストで`tailscale serve --bg 3000`を実行し、tailnetへ公開します。
Tailscaleもコンテナで動かす場合は、アプリをそのコンテナのnetwork namespaceに入れ、ホストへポートをpublishしないでください。
公開インターネットからのGET/POSTはWebSubのcallbackの正確なパスだけへ通します。[callback設定](docs/deploy.md#websub-callback)を参照してください。

tailnetにログインした端末から、ServeのURL（`https://<host>.<tailnet>.ts.net`）を開きます。
最初の認証済みユーザーがマスターユーザーになります。空のDBに既定のアカウントはありません。
配布イメージでも`NODE_ENV=production`の明示が必要です。

## チャンネルを追加して読む

1. 「チャンネル」で`UC`から始まるチャンネルIDを追加します。
   `GIS_CLIENT_ID`を設定すると、ヘッダーメニューの「チャンネル同期 (YouTube)」から購読一覧も取り込めます。
2. フィードへ戻ると、取得した動画が公開日時順に並びます。
3. グループやショート・ライブのフィルターで絞込み、動画のスワイプで非表示にできます。

購読同期はGoogleの購読一覧との差分を反映し、登録の解除も行います。同期するGoogleアカウントを確認してください。

WebSubは公開callbackを通して新着動画を受信します。
APIによる取りこぼし回収と動画詳細の取得には、`YOUTUBE_API_KEY`を設定してください。
定期取得には`CATCHUP_INTERVAL_MINUTES`へ`10`など正の分数も指定します。
キーがなければAPI回収や詳細による動画の分類はできません。
定期取得を無効にすると、起動時・手動更新は上限付きの1巡回だけで止まります。
追加直後は、pushまたはAPIで動画を取得するまで一覧が空の場合があります。

## 動画を見て高評価・コメントする

動画をタップすると、アプリ内のYouTubeプレイヤーで再生します。YouTubeアプリや別のタブは開きません。
閉じるかブラウザで戻ると、一覧の元の位置に戻ります。フィードから開いた動画は「もう見た」の状態です。
Ctrl・Cmdを押しながらのクリックと中クリックは、YouTubeを新しいタブで開きます。

プレイヤーの下でコメントを読み書きできます。送ったコメントは開いている動画に付き、その場で編集・削除も可能です。
プレイヤーの下の親指のボタンで、開いている動画に高評価を付けます。もう一度押すと外れます。
高評価かコメントを初めて送るときに、Googleが書き込みの許可を求めます。

- コメントの閲覧には`YOUTUBE_API_KEY`、高評価とコメントの送信には`GIS_CLIENT_ID`が必要です（[デプロイ手順](docs/deploy.md)）。
- 高評価はYouTube Data APIで送ります。Googleの[APIリファレンス](https://developers.google.com/youtube/v3/docs/videos/rate)は、APIで付けた評価が動画の公式の高評価数に影響しないとしています。
- コメントへの高評価はAPIに無いため、付けられません。
- 投稿者が埋め込みを禁止した動画は再生できません。右上のボタンでYouTubeを開いてください。
- コメントへの返信は表示しません。
- [subfeed-browser](https://github.com/miyabisun/subfeed-browser)（Androidの自作ブラウザ）の中で開くと、動画のタップはアプリへ渡され、YouTube自身の画面で再生します。高評価とコメントもYouTubeの画面で行います。

## チャンネルの投稿を読む

フィードには、登録チャンネルの「投稿」タブにある公開投稿も、動画と時刻順に混ぜて表示します。
フィード上部の「動画」「投稿」で表示する種類を選べます。起動時は両方オンです。
「返信」でYouTubeの投稿画面を開き、そこで返信します。スマホではYouTubeアプリで開く場合があります。

投稿は起動直後から約1時間ごとに、各チャンネルの投稿タブ先頭ページをログインなしで取得します。
YouTubeは投稿の公式APIを提供していないため、非公式のWeb取得を使います。
YouTube側の変更で取得できなくなる場合があります。失敗はログに残り、設定があればDiscordへ通知します。
投稿日時は「3日前」などの表記から推定した値です。メンバー限定の投稿は表示しません。

Googleの認証情報、API予算、任意のDiscord通知は[デプロイ手順](docs/deploy.md)で設定します。
ブラウザの購読同期には公開GISクライアントIDを使います。
クライアントシークレットは不要で、OAuthトークンをこのサーバーへ送信・保存しません。

## データと更新

SQLiteのDBは名前付きvolumeに保存します。コンテナの更新時も同じvolumeを保持してください。
SQLiteのbackup機能を使うか、アプリを停止してデータ用ディレクトリ全体をコピーします。
新しいイメージをpullしたらコンテナだけを停止・削除し、同じ起動コマンドで作り直します。
volumeと環境変数ファイルを引き継ぎ、`.env`や秘密値は公開しないでください。

## ソースからビルドする

バックエンドはRustとSQLite、フロントエンドはSvelte 5とViteです。
Git、Rust 1.96以上、Node.js 22以上、npm、pkg-config、OpenSSL開発用ライブラリを用意します。

```bash
git clone https://github.com/miyabisun/youtube-sub-feed.git
cd youtube-sub-feed
cp .env.example .env
npm --prefix client ci
npm --prefix client run build
cargo build --release --locked
NODE_ENV=production ./target/release/youtube-sub-feed
```

起動前に`.env`を編集します。DBの既定パスは`./feed.db`です。
本番ではDockerと同じTailscale Serveの設定を使います。
ローカル開発用の`./bin/dev`は画面をビルド・監視し、Rustサーバーを起動します。
開発モードはDB内の最初のユーザーを使うため、空のDBでは自動ログインできません。
初期設定と隔離方法は[ローカル開発](docs/deploy.md#ローカル開発)を参照してください。
バックエンドの検証は、画面のビルド後に`cargo fmt --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings`、`cargo test --locked`で行います。

## 環境変数

設定名・必須条件・既定値・不正値の扱いは[英語版の一覧](README.md#environment-variables)を参照してください。
旧`GOOGLE_CLIENT_ID`、`GOOGLE_CLIENT_SECRET`、`GOOGLE_REDIRECT_URI`は使いません。
ブラウザの同期は`GIS_CLIENT_ID`へ移行しており、サーバー用secretやredirectの代替変数はありません。
