# 06: ServiceAccount APIエンドポイント実装

## 概要

`core/v1` APIグループにServiceAccountのCRUDエンドポイントを追加する。

## 作業内容

### エンドポイントファイル

`tugboat-apiserver/src/endpoints/v1_core/service_account.rs` を作成。

### エンドポイント (namespaced)

- `GET    /v1/serviceaccounts` - list-all
- `GET    /v1/namespaces/{namespace}/serviceaccounts` - list
- `POST   /v1/namespaces/{namespace}/serviceaccounts` - create
- `GET    /v1/namespaces/{namespace}/serviceaccounts/{name}` - read
- `PUT    /v1/namespaces/{namespace}/serviceaccounts/{name}` - update
- `PATCH  /v1/namespaces/{namespace}/serviceaccounts/{name}` - patch
- `DELETE /v1/namespaces/{namespace}/serviceaccounts/{name}` - delete

### 登録

- `v1_core/mod.rs` に `register_service_account()` を追加
- `resource_registry.rs` の `all_resource_apis()` にServiceAccountの `ResourceApiDescriptor` を追加
- OpenAPIドキュメントに追加

## 確認

```bash
cargo build --package tugboat-apiserver
cargo clippy --package tugboat-apiserver
```

## 参考

- 既存のnamespacedリソース: `v1_core/configmap.rs` や `v1_core/secret.rs` のパターンを踏襲
- 汎用ハンドラ: `create_namespaced<ServiceAccount>` 等を利用
