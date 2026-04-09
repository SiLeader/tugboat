# 09: 認可ミドルウェアのAPI統合

## 概要

08で実装した認可エバリュエーターをAPIサーバーのリクエスト処理パイプラインに統合する。

## 設計

### ミドルウェア or ハンドラーレベル統合

2つのアプローチがある:

**案A: actix-webミドルウェア**
- 全リクエストに対してURLパスとHTTPメソッドからリソース・verb情報を抽出
- 認可判定後、許可されたリクエストのみハンドラーに転送
- 長所: ハンドラーコードの変更が不要
- 短所: URLパースのロジックが重複

**案B: ハンドラーレベルのextractor** (推奨)
- actix-webの `FromRequest` トレイトを実装した `AuthorizedRequest<T>` extractor
- ハンドラーの引数として受け取り、認可チェックを自動実行
- 長所: リソース情報がハンドラーのコンテキストから自然に取得可能
- 短所: 全ハンドラーの引数を更新する必要がある

**案C: ミドルウェア + resource_registry連携** (推奨)
- URLパターンから resource_registry の情報を引いてverb/resource/namespaceを特定
- 汎用ミドルウェアで認可チェック
- 長所: ハンドラー修正不要、resource_registryと一貫性がある

## 作業内容

1. `tugboat-apiserver/src/auth/middleware.rs` に認可ミドルウェアを追加
2. URLパスからAPIグループ・リソース・namespace・verb情報を抽出するロジック
3. `tugboat-apiserver/src/lib.rs` のミドルウェアスタックに認可を追加
4. `StatusResponse::forbidden()` / `StatusResponse::unauthorized()` のコメントアウトを解除
5. 認可のバイパス設定を追加 (e.g., healthzエンドポイント、API discovery)

### 認可バイパスリスト

以下のエンドポイントは認可チェックをスキップ:
- `/healthz` - ヘルスチェック
- `/apis` - API discovery
- `/openapi.json` - OpenAPIスキーマ

### 設定

```toml
[authorization]
mode = "RBAC"    # "RBAC" | "AlwaysAllow"
```

`AlwaysAllow` モードは開発・テスト時に認可を無効にするために使用。

## 確認

```bash
cargo build --package tugboat-apiserver
cargo test --package tugboat-apiserver
```

## 注意

- 既存の統合テストが壊れないよう、デフォルトは `AlwaysAllow` にし、明示的にRBACモードを有効にする形が安全
- 段階的にRBACモードへの移行パスを提供

## 参考

- resource_registry.rs: `all_resource_apis()` からリソースメタデータを取得
- actix-web Service/Transform トレイト
