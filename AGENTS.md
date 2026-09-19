# youtube-sub-feed プロジェクトルール

開発・テスト・レビューは、共通の `deliver`・`test-verify`・`change-review` skillに従う。

- デプロイ手順を変更したら、[docs/deploy.md](docs/deploy.md)を更新する。
- Rustの仕様は `src/*.rs` のインラインテスト（`#[cfg(test)]`）で表す。
  テスト名は振る舞いを説明し、境界値・異常系も確認する。
- `client/` 配下の変更後は、`cd client && npx vite build` の成功を確認する。
