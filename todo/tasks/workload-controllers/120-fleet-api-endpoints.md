# Fleet API エンドポイントの実装

## 概要

`tugboat-apiserver` に Fleet の CRUD エンドポイントを追加する。
Deployment/ReplicaSet と同じ namespaced リソースのパターンで実装する。

## 対象ファイル

- `tugboat-apiserver/src/endpoints/v1_apps/fleet.rs`（新規作成）
- `tugboat-apiserver/src/endpoints/v1_apps/mod.rs`（登録）

## エンドポイント一覧

| Method | Path | 処理 |
|--------|------|------|
| POST   | `/apis/apps/v1/namespaces/{namespace}/fleets` | 作成 |
| GET    | `/apis/apps/v1/namespaces/{namespace}/fleets` | 名前空間内一覧 |
| GET    | `/apis/apps/v1/fleets` | 全名前空間一覧 |
| GET    | `/apis/apps/v1/namespaces/{namespace}/fleets/{name}` | 取得 |
| PUT    | `/apis/apps/v1/namespaces/{namespace}/fleets/{name}` | 置換 |
| PATCH  | `/apis/apps/v1/namespaces/{namespace}/fleets/{name}` | パッチ |
| DELETE | `/apis/apps/v1/namespaces/{namespace}/fleets/{name}` | 削除 |

## 実装内容

既存の `deployment.rs` または `replicaset.rs` を参考に、
`resource_handlers` の汎用ハンドラーを使って実装する。

### fleet.rs の骨格

```rust
use tugboat_resources::manifests::apps::v1::Fleet;
// Deployment や ReplicaSet と同じ generic ハンドラーを利用
pub fn fleet_routes() -> actix_web::Scope {
    // ...
}
```

### mod.rs への登録

```rust
mod fleet;
pub use fleet::fleet_routes;
// scope に追加
```

## 完了条件

- `cargo build --release --package tugboat-apiserver` が通ること
- `cargo clippy` で警告がないこと
- OpenAPI スキーマ（`schema` feature が有効なとき）に Fleet が含まれる
