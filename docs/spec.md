# youtube-sub-feed 仕様書

## 概要

YouTubeの登録チャンネルの最新動画を公開日時の降順で一覧表示するWebアプリ。
YouTubeトップページのレコメンド汚染を回避し、自分が登録したチャンネルの動画だけを閲覧する。

- URL: `https://feed.sis.jp`
- 利用者: 自分のみ（認証で制限）

## 技術スタック

novel-server に準拠（詳細: [reference-novel-server.md](./reference-novel-server.md)）:

| 項目 | 技術 |
|------|------|
| ランタイム | Rust (tokio) |
| バックエンド | axum |
| データベース | SQLite (rusqlite) |
| ORM | なし（raw rusqlite） |
| フロントエンド | Svelte 5 + Vite |
| スタイリング | Sass |
| 言語 | Rust |
| コンテナ | Docker (3段マルチステージビルド) |

## 認証

### Google OAuth2

- Google Cloud Console でOAuth2クライアントを作成（**テストモード**で運用）
- スコープ:
  - `https://www.googleapis.com/auth/youtube.readonly`（登録チャンネル・動画情報の読み取り）
- 認証フロー: Authorization Code Flow
- コールバックURL:
  - 本番: `https://feed.sis.jp/api/auth/callback`
  - 開発: `http://localhost:3000/api/auth/callback`
- トークン管理:
  - アクセストークン・リフレッシュトークンをDBに保存
  - アクセストークン期限切れ時はリフレッシュトークンで自動更新
  - YouTube API 呼び出し前に `token_expires_at` をチェックし、期限切れなら事前にリフレッシュ
- セッション管理:
  - Cookie ベースのセッション（HttpOnly, SameSite=Lax）
  - Cookie 名: `session`
  - セッションID生成: UUID v4
  - 本番: `Secure` フラグ有効（HTTPS）、開発: `Secure` 無効（localhost HTTP）
  - セッション有効期限: 30日
  - セッション期限切れ時: ログインページへ自動リダイレクト
- 利用者制限:
  - Google OAuth2 の**テストモード**で制限（テストユーザーとして登録したアカウントのみログイン可能）
  - アプリ側での追加のメールアドレス制限は行わない

### 認証ミドルウェア

- `/api/auth/login` と `/api/auth/callback` は認証不要
- それ以外の全 `/api/*` エンドポイントに認証ミドルウェアを適用
- フロントエンドの静的ファイル (`client/build/`) は認証なしで配信
- 未認証の API リクエストには `401 Unauthorized` を返す

### OAuth2 コールバック後の遷移

- callback でセッション Cookie を設定後、フロントの `/` にリダイレクト
- フロントエンドは `GET /api/auth/me` でログイン状態を確認
- 未ログイン（401）の場合、フロント側で `/login` にリダイレクト

## YouTube Data API v3

### 使用エンドポイント

| エンドポイント | クォータコスト | 用途 |
|---|---|---|
| Subscriptions.list | 1 | 登録チャンネル一覧の取得 |
| PlaylistItems.list | 1 | チャンネルの最新動画取得（UUプレイリスト） |
| Videos.list | 1 | 動画の詳細情報（duration等）のバッチ取得 |

**使用しないエンドポイント:**
- Search.list（100ユニット/回、コスト高すぎ）

### RSS-First 戦略（クォータ削減）

YouTube の公開 RSS フィード（`https://www.youtube.com/feeds/videos.xml?channel_id={id}`）はクォータを消費しない。各チャンネルの巡回時にまず RSS で新着を検知し、新着があるチャンネルのみ YouTube Data API を呼ぶハイブリッド方式を採用する。

- **RSS チェック → 新着なし**: API スキップ、`last_fetched_at` のみ更新して次の tick へ
- **RSS チェック → 新着あり**: 従来通り API で動画詳細を取得
- **RSS 取得失敗**: 安全側に倒して API フォールバック（`hasNewVideos: true` として扱う）
- **初回取得**（`last_fetched_at IS NULL`）: RSS をスキップし、常に API で取得
- **手動リフレッシュ**（`POST /api/channels/:id/refresh`）: RSS を介さず常に API 直接
- **ライブ察知ループ**: RSS を使わず常に API 直叩き（5分/周で新着 + ライブ終了検知）

### クォータ予算（10,000ユニット/日）

```
■ 新着検知（200ch、15分/周、RSS-First）
  ※ 新着動画があるチャンネルのみ API 呼び出し
  ※ 平均 ~0.3 動画/日/ch → 200ch × 0.3 ≒ 65 新着/日
  PlaylistItems.list: 65                          =    65ユニット
  Videos.list:        65                          =    65ユニット
  小計                                            ≒   130ユニット/日

■ ライブ察知（5ch、5分/周、API直叩き）
  PlaylistItems.list: 5 × 288回                   = 1,440ユニット
  Videos.list（新着）:                             ≒     5ユニット
  Videos.list（終了検知、配信中1本想定）:           ≒   288ユニット
  小計                                            ≒ 1,733ユニット/日

■ その他
  Subscriptions.list: 10分ごと × 144回 × 4ページ  =   576ユニット
  UUSH照合:           ショート判定                 ≒    20ユニット
  手動リフレッシュ:   数回/日                      ≒    10ユニット
  初回セットアップ:   1回のみ                      ≒   200ユニット
  小計                                            ≒   606ユニット/日

■ 合計: 約 2,469ユニット/日（初回除く、ライブ配信中1本想定）
  ※ クォータ上限 10,000 の 25% 程度で運用可能
```

### 動画取得の仕組み

1. チャンネルIDの `UC` プレフィックスを `UU` に置換 → アップロードプレイリストIDを取得
2. `PlaylistItems.list` で各チャンネルの最新動画を取得（maxResults=10）
   - レスポンスに含まれるタイトル・サムネイルで**既存レコードも上書き更新**（投稿者による修正に追従）
3. DB未登録の新着動画のみ `Videos.list` でバッチ取得（最大50件/リクエスト）
   - `part=snippet,contentDetails,liveStreamingDetails` を指定
   - `contentDetails.duration` → 再生時間
   - `liveStreamingDetails` の存在有無 → ライブ配信判定（追加クォータなし）
   - `liveStreamingDetails.actualEndTime` → `livestream_ended_at` に保存（NULL なら配信中）
   - duration, is_short は変更されないため、新着時のみ取得すれば十分
   - ライブ配信の終了検知: ライブ察知ループ（5分/周）で `is_livestream=1 AND livestream_ended_at IS NULL` の動画を Videos.list で再取得し、終了日時を更新
4. 新着動画のうち duration ≤ 3分（180秒）のものについて、ショート動画判定を実施
   - 2024年10月15日に YouTube Shorts の上限が 60秒 → 3分に拡大
   - チャンネルIDから `UUSH` プレイリスト（Shorts専用）を `PlaylistItems.list` で取得
   - そのプレイリストに含まれていれば `is_short = 1`
   - `UUSH` は非公式機能のため、取得失敗時は `is_short = 0`（通常動画扱い）
   - UUSH プレイリストの取得結果はチャンネル単位でメモリキャッシュし、同一巡回内での重複取得を防止
   - キャッシュは新着検知ループ1周完了ごとにクリア

### 更新戦略

2つの巡回ループを並行稼働させる（novel-server のラウンドロビン同期パターンに準拠）:

- **新着検知ループ（15分/周）**: `show_livestreams=0` のチャンネルを RSS-First で巡回
  - `interval = 15 * 60 * 1000 / channel_count` で均等間隔にスケジューリング
  - RSS で新着なしと判定されたチャンネルは API をスキップ
  - YouTube RSS フィードの約15分キャッシュに合わせた周期
  - ライブチェックはしない（ライブ察知ループの責務）
- **ライブ察知ループ（5分/周）**: `show_livestreams=1` のチャンネルのみ（最大5件）、**API直叩き**
  - `interval = 5 * 60 * 1000 / fast_channel_count` で均等間隔
  - RSS をスキップし、常に API で動画取得（ライブ開始の即時検知）
  - 毎 tick で `check_livestreams` を実行（ライブ終了検知）
- 各ループは `tokio::time::sleep` チェーンで1チャンネルずつ処理し、1周完了後に次の周を開始
- **手動リフレッシュ**: 指定した1チャンネルのみを即座に更新（`POST /api/channels/:id/refresh`）— RSS を介さず常に API 直接
- **登録チャンネルリストの同期**: 10分ごと（RSS-First により日次クォータに十分な余裕があるため、新規登録を即座に反映）

#### 巡回順序

- 新着検知・ライブ察知それぞれのループ内で `last_fetched_at` が最も古いチャンネルから順に処理（`ORDER BY last_fetched_at ASC`）
- サーバー起動と同時に巡回を開始。ただし有効なアクセストークンが存在しない場合（初回起動時等）はログイン完了を待機
- 巡回処理はトランザクション外で YouTube API を呼び出し、結果の DB 書き込みのみ短いトランザクションで実行

#### 初回セットアップ

- 初回ログイン時（channels テーブルが空の状態）に Subscriptions.list で登録チャンネルを即時取得
- 全チャンネルの動画取得を即座に実行（バックグラウンド巡回に任せない）
- 完了後に Discord へ通知（サーバー再起動時は channels が既に存在するため通知しない）

#### エラーハンドリング・リトライ

- API 失敗時は最大3回リトライ（1秒 → 2秒 → 3秒の線形バックオフ）
- API エラー発生時は当該チャンネルをスキップして次のチャンネルに進む（スタックループ防止）
- クォータ超過時の検知: HTTP 403 + `reason: "quotaExceeded"` で判定
- クォータ超過を検知したら巡回ループを停止し、太平洋時間 午前0時（日本時間 17:00）まで待機してから再開

### チャンネル登録解除時の扱い

- YouTubeで登録解除したチャンネルは、次回 Subscriptions.list 同期（10分ごと）時に**物理削除**
- そのチャンネルの動画データも**物理削除**（CASCADE）
- 理由: 論理削除だとフィードに解除済みチャンネルの動画が残る。再登録時に非表示済み動画まで復活するのも望ましくない
- 巡回中に 404 `playlistNotFound` を検知した場合は**スキップ**し、Discord に警告通知を送信（動画を全削除したチャンネル等。削除→再追加ループを防止）

### ライブ配信の扱い

- デフォルトではライブ配信（アーカイブ含む）はフィードに**表示しない**
- チャンネルごとに `show_livestreams=1` を設定可能
  - 友人のチャンネル等、個別に許可する運用
  - 有効にするとフィードへのライブ表示 + ライブ察知巡回（5分/周、API直叩き）の両方が適用される
  - 現時点で1件、最大5件程度の運用を想定
- カード上に「LIVE」または「配信アーカイブ」ラベルを表示

### ショート動画の扱い

- 通常動画と混ぜてフィードに表示
- カード上に「Shorts」ラベルを表示して区別
- 判定方法: UUSH プレイリスト照合（非公式、取得失敗時は通常動画扱い）
- リンク先: `https://www.youtube.com/shorts/VIDEO_ID`（通常動画とは異なるURL）

### キャッシュ・データ永続化

取得した動画情報はSQLiteに永続化し、APIキャッシュとして機能させる。

- **動画メタデータ**: PlaylistItems.list の定期巡回で既存動画のタイトル・サムネイルを更新（クォータ追加消費なし）
  - UPSERT の `DO UPDATE` に WHERE 差分チェックを付与し、値が変わった動画のみ書き込み（無駄な書き込みによる断片化・WAL肥大化を防止）
  - Videos.list は DB未登録の新着動画のみ呼び出し（duration取得用）
  - これにより Videos.list の呼び出しは新着動画分のみに抑制
- **チャンネル情報**: DBに保存
- **データ保持方針**: 削除せず蓄積（適切なインデックスで性能を維持）
- **フロントエンド**: APIレスポンスをインメモリキャッシュ（novel-server同様のTTLベース Map キャッシュ）

## データベーススキーマ

### channels テーブル

| カラム | 型 | 説明 |
|--------|-----|------|
| id | TEXT | チャンネルID (UC...) PK |
| title | TEXT | チャンネル名 |
| thumbnail_url | TEXT | チャンネルアイコンURL |
| upload_playlist_id | TEXT | アップロードプレイリストID (UU...) |
| show_livestreams | INTEGER | ライブ配信をフィードに表示 + ライブ察知巡回対象 (0: 非表示, 1: 表示/5分API直叩き) DEFAULT 0 |
| last_fetched_at | TEXT | 最終取得日時 (ISO 8601) |
| created_at | TEXT | 登録日時 |

### videos テーブル

| カラム | 型 | 説明 |
|--------|-----|------|
| id | TEXT | 動画ID PK（YouTubeの一意ID、URLに使用） |
| channel_id | TEXT | チャンネルID FK |
| title | TEXT | 動画タイトル |
| thumbnail_url | TEXT | サムネイルURL |
| published_at | TEXT | 公開日時 (ISO 8601) |
| duration | TEXT | 再生時間 (ISO 8601 duration, 表示時は `1:02:03` 形式にフォーマット) |
| is_short | INTEGER | ショート動画か (0: 通常, 1: Shorts) DEFAULT 0 |
| is_livestream | INTEGER | ライブ配信か (0: 通常動画, 1: ライブ) DEFAULT 0 |
| livestream_ended_at | TEXT | ライブ配信終了日時 (ISO 8601)。NULL=未終了または通常動画 |
| is_hidden | INTEGER | 論理削除フラグ (0: 表示, 1: 非表示) DEFAULT 0 |
| fetched_at | TEXT | 取得日時 |

**FK制約**: `channel_id` → `channels.id` ON DELETE CASCADE

**インデックス**:
- `idx_videos_published` on `(published_at DESC)`
- `idx_videos_channel` on `(channel_id)`
- `idx_videos_hidden` on `(is_hidden, published_at DESC)` — フィード表示用の複合インデックス

### groups テーブル

| カラム | 型 | 説明 |
|--------|-----|------|
| id | INTEGER | グループID PK (autoincrement) |
| name | TEXT | グループ名 |
| sort_order | INTEGER | 表示順 |
| created_at | TEXT | 作成日時 |

### channel_groups テーブル（多対多）

| カラム | 型 | 説明 |
|--------|-----|------|
| channel_id | TEXT | チャンネルID FK |
| group_id | INTEGER | グループID FK |

**複合PK**: `(channel_id, group_id)`
**FK制約**: `channel_id` → `channels.id` ON DELETE CASCADE, `group_id` → `groups.id` ON DELETE CASCADE

### auth テーブル

| カラム | 型 | 説明 |
|--------|-----|------|
| id | INTEGER | PK (autoincrement) |
| google_id | TEXT | GoogleアカウントID |
| email | TEXT | メールアドレス |
| access_token | TEXT | アクセストークン |
| refresh_token | TEXT | リフレッシュトークン |
| token_expires_at | TEXT | トークン有効期限 |
| updated_at | TEXT | 更新日時 |

### sessions テーブル

| カラム | 型 | 説明 |
|--------|-----|------|
| id | TEXT | セッションID PK |
| auth_id | INTEGER | auth.id FK |
| expires_at | TEXT | セッション有効期限 |
| created_at | TEXT | 作成日時 |

**FK制約**: `auth_id` → `auth.id` ON DELETE CASCADE

## API エンドポイント

### 認証

- `GET /api/auth/login` — Google OAuth2 ログインへリダイレクト
- `GET /api/auth/callback` — OAuth2 コールバック
- `POST /api/auth/logout` — ログアウト（セッション破棄）
- `GET /api/auth/me` — ログイン状態確認

### 動画フィード

- `GET /api/feed` — 動画一覧（公開日時降順）
  - フィルタ条件: `is_hidden=0 AND (is_livestream=0 OR channels.show_livestreams=1)`
  - クエリパラメータ: `?group=<group_id>` でグループフィルタ
  - クエリパラメータ: `?limit=100&offset=0` でページネーション
- `PATCH /api/videos/:id/hide` — 動画を論理削除（非表示化）
- `PATCH /api/videos/:id/unhide` — 非表示動画を復元

### チャンネル

- `GET /api/channels` — 登録チャンネル一覧
- `GET /api/channels/:id/videos` — 特定チャンネルの動画一覧（非表示含む、チャンネル詳細ページ用）
  - クエリパラメータ: `?limit=100&offset=0` でページネーション
- `POST /api/channels/sync` — YouTube から登録チャンネルを再同期
- `POST /api/channels/:id/refresh` — 指定チャンネルのみ手動更新
- `PATCH /api/channels/:id` — チャンネル設定更新（`show_livestreams`）

### グループ管理

- `GET /api/groups` — グループ一覧
- `POST /api/groups` — グループ作成
- `PATCH /api/groups/:id` — グループ更新（名前）
- `PUT /api/groups/reorder` — グループ並び替え（`body: { order: [3, 1, 2] }` — group_id の配列、インデックス順に sort_order を割り当て）
- `DELETE /api/groups/:id` — グループ削除
- `PUT /api/groups/:id/channels` — グループにチャンネルを設定（全置換）（`body: { channelIds: ["UC..."] }`）

## フロントエンド

### ページ構成

| パス | ページ | 説明 |
|------|--------|------|
| `/` | フィード | 全チャンネルの最新動画一覧 |
| `/group/:id` | グループフィード | 特定グループの動画一覧 |
| `/channels` | チャンネル一覧 | 登録チャンネルの一覧・検索 |
| `/channel/:id` | チャンネル詳細 | 1チャンネルの動画一覧（非表示動画の管理） |
| `/login` | ログイン | Google ログインボタン |
| `/settings` | 管理画面 | グループ管理・チャンネル割り当て |

### ヘッダー

- novel-server のヘッダーを参考にしたタブナビゲーション
- タブ構成:
  - グループタブ: 「すべて」（デフォルト）+ 各グループ → フィードの絞り込み
  - 右側リンク: 「CH」→ チャンネル一覧ページ、「設定」→ 管理画面
- グループタブが多い場合は横スクロールで対応
- モバイル（800px未満）: グループタブをドロップダウン `<select>` に切り替え

### フィードページ（トップ）

- デフォルト表示: 全登録チャンネルの動画を公開日時の降順で表示
- **無限スクロール**: スクロール末尾に到達したら次の100件を自動読み込み
- ヘッダーのグループタブで `/group/:id` に遷移して絞り込み
- 動画カードレイアウト:
  - **縦一列リスト**（サムネイル上・情報下の上下配置）
  - 視線のZ移動を避け、一覧性を重視
- 動画カード表示項目:
  - サムネイル画像
  - 動画タイトル
  - チャンネル名（タップでチャンネル詳細ページへ遷移）
  - 再生時間
  - 公開日時（相対表記: 「○分前」「○時間前」「○日前」「○ヶ月前」「○年前」）
  - ラベル（該当時のみ表示）:
    - 「Shorts」— ショート動画
    - 「LIVE」— ライブ配信中（`is_livestream=1 AND livestream_ended_at IS NULL`）
    - 「配信アーカイブ」— 終了済みライブ配信（`is_livestream=1 AND livestream_ended_at IS NOT NULL`）
- 動画カードタップ → YouTubeへ遷移
  - 通常動画: `https://www.youtube.com/watch?v=VIDEO_ID`
  - ショート: `https://www.youtube.com/shorts/VIDEO_ID`
  - スマホ: OS の Universal Links / App Links により YouTube アプリで開く
  - PC: ブラウザで YouTube を開く
  - 特別な実装は不要（通常の `<a>` タグでOK）
- 動画の非表示操作:
  - カード右上の「もう見た」ボタンをクリック/タップで論理削除（非表示化）
  - **スマホ**: ボタンを `opacity: 0.7` で常時表示
  - **PC**: ホバー時にボタンを表示（`opacity: 0 → 0.7 → 1`）
  - `is_hidden = 1` に更新し、フィードから消える
  - 興味のない動画を手動で消せる機能
- スワイプナビゲーション（スマホ）:
  - 左スワイプで次のグループへ、右スワイプで前のグループへ遷移
  - 遷移サイクル: TOP → グループ1 → グループ2 → ... → TOP（循環）
  - 閾値: 50px

### チャンネル一覧ページ (`/channels`)

- 登録チャンネルをリスト表示（アイコン、チャンネル名、所属グループ）
- チャンネル名で検索・フィルタ
- タップでチャンネル詳細ページへ遷移

### チャンネル詳細ページ (`/channel/:id`)

- 対象チャンネルの全動画を表示（非表示動画も含む）
- 非表示動画は禁止マーク（丸にスラッシュ）を表示して区別
- 非表示/復元操作:
  - **スマホ**: 左スワイプで非表示化、右スワイプで非表示から復元
  - **PC**: カード内の非表示/復元ボタンをクリック
- チャンネル設定（ライブ配信の切り替え — 有効にすると5分間隔のAPI直叩きライブ察知も適用）
- YouTube チャンネルページへの外部リンク（別タブで開く）

### 管理画面

- 全画面幅レイアウト（`max-width` 制約なし）
- グループのCRUD操作
- グループの並び替え: カード左端のハンドルを HTML5 Drag and Drop API でドラッグして上下に移動（タッチ未対応）
- チャンネル割り当て:
  - 各チャンネルカード: チェックボックス、アイコン(32px)、チャンネル名、所属グループラベル、YTリンク（YouTube チャンネルページへ新タブ）
  - カードクリックでアコーディオン展開: 最新動画3件のサムネイルを横並び表示（遅延ロード＋キャッシュ）
  - サムネイルクリックで YouTube 動画ページへ新タブ遷移
- チャンネルは複数グループに所属可能

### スワイプ操作

- novel-server のカスタム実装を踏襲（外部ライブラリ不使用）
- `touchstart` / `touchmove` / `touchend` による自前実装
- 2種類のスワイプアクション:
  - **`swipeable`**: 個別アイテム操作用（チャンネル詳細ページで使用）。translateX アニメーション + swipeBg 背景表示付き
    - 確定閾値: 40px、最大移動: 80px
    - リセットアニメーション: `0.2s ease`
    - スワイプ後の誤クリック防止（`preventClick`）
  - **`swipeNav`**: ページ遷移用（フィードページで使用）。方向検出 → コールバック発火のみ（視覚エフェクトなし）
    - 確定閾値: 50px
- 共通仕様:
  - 5px デッドゾーン（ジッター除去）
  - 水平/垂直の方向ロック（`Math.abs(dx) > Math.abs(dy)` で判定）
- スマホ / PC の判定: ブレークポイント 600px で CSS 切り替え（スワイプは `touchstart` イベントの有無で判定）
- PC では同等の機能をカード内ボタンで提供（スワイプ操作なし）

### トースト通知

- 画面下部に表示するフィードバック UI
- 成功時: 生存時間 0.5秒（操作完了の確認）
- エラー時: 生存時間 3秒（内容を読む時間を確保）
- 用途: 非表示化・復元・設定変更等の操作結果通知

### ローディング

- データ取得中はスピナーを表示

## デザイン

### テーマ

- **ダークモードをデフォルト**（ライトモード対応は不要）
- novel-server のデザインシステムを踏襲（詳細: [reference-novel-server.md](./reference-novel-server.md) のデザインシステム節）
  - LiftKit ベースの黄金比 (φ ≈ 1.618) スケーリング
  - スペーシング・フォントサイズ・ボーダーラジアスの CSS トークン体系をそのまま採用
  - セマンティックカラートークン (`--c-bg`, `--c-surface`, `--c-text` 等) をダーク配色で定義
- ダーク配色例:
  - 背景: `#0f0f0f`（YouTube Dark風）
  - カード面: `#1a1a1a`
  - テキスト: `#f1f1f1`
  - サブテキスト: `#aaaaaa`
  - アクセント: `#d93025`（Google UI レッド）

### レイアウト

- スマホファースト設計（主要利用端末）
- 動画カードは**縦一列リスト**（一覧性重視）
- ブレークポイント: **600px**（スマホ / PC の境界）
- PC表示時はコンテンツ幅に max-width を設けて中央寄せ

## Discord 通知

Discord Webhook を使って通知チャンネルへ Embed を POST する。

- **メッセージ形式**: Webhook Embed
  - チャンネル名（author）、動画タイトル（title、リンク付き）、サムネイル（image）、公開日時（timestamp）
  - サイドバーの色にアクセントカラーを設定
- 通知タイミング:
  - **初回セットアップ完了時**: 全チャンネルの初回動画取得が完了した旨を通知
  - **新着動画検知時**: 新しい動画を検知したら通知（1件ずつ送信）
- 環境変数 `DISCORD_WEBHOOK_URL` で Webhook URL を設定（省略時は通知無効）
- エラーハンドリング: 通知失敗時は fire-and-forget（ログ出力のみ、巡回は続行）

## テスト

Rust の `#[test]` / `#[tokio::test]` を使用。
**TDD（テスト駆動開発）** で進める — 実装より先にテストを書き、Red → Green → Refactor のサイクルを回す。

### 方針

- テスト項目は実装時にコードと一緒に設計する（事前に網羅的なリストは作らない）
- エッジケース・異常系・状態遷移はTDDのサイクル中に発見し、テストとして追加していく
- 純粋関数（ビジネスロジック）のユニットテスト + DBロジックテスト（インメモリSQLite）の構成
- HTTP依存コードはオーケストレーション/パススルーが主でありモックはトートロジーになるため、HTTPモックは導入しない

### テスト配置

- 各 `.rs` ファイル末尾に `#[cfg(test)] mod tests { ... }` で配置
- DB テストは `db::open_memory()` でインメモリ SQLite を使用

## 開発環境

### 環境変数

```env
PORT=3000
DATABASE_PATH=./feed.db
GOOGLE_CLIENT_ID=xxx
GOOGLE_CLIENT_SECRET=xxx
GOOGLE_REDIRECT_URI=http://localhost:3000/api/auth/callback
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/xxx/xxx
```

### 開発フロー

- Windows から SSH で Arch Linux に接続し Claude Code を使用
- SSH ポートフォワードで `localhost:3000` にアクセスして動作確認
- novel-server と同じ dev/build/start スクリプト構成

### ログ出力

- `tracing` + `tracing-subscriber` による構造化ログ（Docker ログで確認）
- 巡回状況・エラー・クォータ超過等の重要イベントをログ出力

### DB マイグレーション

- `CREATE TABLE IF NOT EXISTS` で起動時に自動作成（`src/db.rs`）
- 個人用アプリのためロールバック不要

## デプロイ

- VPS 上で Docker コンテナとして稼働
- リバースプロキシ (nginx) で `feed.sis.jp` → コンテナにルーティング
- HTTPS は nginx 側 (Let's Encrypt) で終端
