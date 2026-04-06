# タスク: RuntimeClass API エンドポイントの追加

## 前提

タスク `01-proto-definition.md` が完了済みであること（`RuntimeClass` 型が `tugboat-resources` でビルドできる状態）。

## 概要

`tugboat-apiserver` に `RuntimeClass` のCRUDエンドポイントを追加する。
クラスタスコープリソースのため、`ShipClass` の実装（`shipclass.rs`）を参考にする。

## 実装内容

### 1. エンドポイントファイルの作成

`tugboat-apiserver/src/endpoints/v1_core/runtime_class.rs` を新規作成する。

`shipclass.rs` と同様のパターンで以下のハンドラを実装する:

- `handle_runtimeclass_create` → `POST /api/v1/runtimeclasses`
- `handle_runtimeclass_delete` → `DELETE /api/v1/runtimeclasses/{name}`
- `handle_runtimeclass_list` → `GET /api/v1/runtimeclasses`
- `handle_runtimeclass_read` → `GET /api/v1/runtimeclasses/{name}`

各ハンドラは `resource_handlers::create_cluster`、`resource_handlers::delete_resource`、
`resource_handlers::list_resources`、`resource_handlers::read_resource` を使用する。

### 2. mod.rs への登録

`tugboat-apiserver/src/endpoints/v1_core/mod.rs` を以下の3箇所で更新する:

#### a) モジュール宣言の追加

```rust
mod runtime_class;
```

#### b) `#[openapi(...)]` の `paths` セクションへの追加

```rust
runtime_class::handle_runtimeclass_create,
runtime_class::handle_runtimeclass_delete,
runtime_class::handle_runtimeclass_list,
runtime_class::handle_runtimeclass_read,
```

#### c) `components(schemas(...))` セクションへの追加

```rust
tugboat_resources::manifests::core::v1::RuntimeClass,
```

#### d) `register_runtimeclass` 関数の追加と `register_v1_core` への組み込み

```rust
pub(super) fn register_runtimeclass(service: &mut ServiceConfig) {
    service
        .service(runtime_class::handle_runtimeclass_create)
        .service(runtime_class::handle_runtimeclass_delete)
        .service(runtime_class::handle_runtimeclass_list)
        .service(runtime_class::handle_runtimeclass_read);
}
```

`register_v1_core` に `.configure(register_runtimeclass)` を追加する。

## 確認コマンド

```bash
cargo build --package tugboat-apiserver
cargo clippy --package tugboat-apiserver
```

手動動作確認（APIサーバー起動後）:
```bash
# 作成
curl -X POST http://localhost:8080/api/v1/runtimeclasses \
  -H 'Content-Type: application/json' \
  -d '{"apiVersion":"core/v1","kind":"RuntimeClass","metadata":{"name":"qemu-kvm"},"spec":{"liveMigration":true,"hotplug":{"cpu":{"add":true,"remove":false}}}}'

# 一覧
curl http://localhost:8080/api/v1/runtimeclasses

# 取得
curl http://localhost:8080/api/v1/runtimeclasses/qemu-kvm

# 削除
curl -X DELETE http://localhost:8080/api/v1/runtimeclasses/qemu-kvm
```
