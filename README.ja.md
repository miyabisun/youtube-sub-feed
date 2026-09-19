# youtube-sub-feed

[English](README.md)

YouTubeの登録チャンネルの動画を、公開日時順に閲覧するWebアプリです。
チャンネルの手動追加や購読一覧の取込み、グループ分け、動画の非表示、ショート・ライブの絞込みに対応します。

## Dockerで起動する

Linux amd64で動くDocker、公開ドメイン、ログイン用のCloudflare Accessが必要です。
アプリはAccessから届くメールアドレスのヘッダーを信頼し、その正当性を自分では検証しません。
サーバーのポートへ直接到達できないようにし、利用者をAccess経由で接続させてください。
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

この例は同じホスト上のTunnel接続プログラムから`http://localhost:3000`へ転送します。
接続プログラムもコンテナで動かす場合は、非公開のDockerネットワークで接続してください。
WebSubのcallbackだけはAccessログインなしでGET/POSTを受け付ける必要があります。
他の画面・APIは保護を維持します。[callback設定](docs/deploy.md#websub-callback)を参照してください。

公開URLを開きます。Accessが許可したメールアドレスでログインしてください。
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
本番ではDockerと同じAccess設定を使います。
ローカル開発用の`./bin/dev`は画面をビルド・監視し、Rustサーバーを起動します。
開発モードはDB内の最初のユーザーを使うため、空のDBでは自動ログインできません。
初期設定と隔離方法は[ローカル開発](docs/deploy.md#ローカル開発)を参照してください。
バックエンドの検証は、画面のビルド後に`cargo test --locked`で行います。

## 環境変数

設定名・必須条件・既定値・不正値の扱いは[英語版の一覧](README.md#environment-variables)を参照してください。
旧`GOOGLE_CLIENT_ID`、`GOOGLE_CLIENT_SECRET`、`GOOGLE_REDIRECT_URI`は使いません。
ブラウザの同期は`GIS_CLIENT_ID`へ移行しており、サーバー用secretやredirectの代替変数はありません。
