# 05: RBAC APIエンドポイント実装

## 概要

`authorization/v1` APIグループの4リソース (Role, ClusterRole, RoleBinding, ClusterRoleBinding) に対するCRUDエンドポイントを実装する。

## 作業内容

### ディレクトリ構成

```
tugboat-apiserver/src/endpoints/v1_authorization/
├── mod.rs
├── role.rs
├── cluster_role.rs
├── role_binding.rs
└── cluster_role_binding.rs
```

### 各リソースのエンドポイント

**ClusterRole, ClusterRoleBinding (cluster-scoped)**:
- `GET    /apis/authorization/v1/{plural}` - list
- `POST   /apis/authorization/v1/{plural}` - create
- `GET    /apis/authorization/v1/{plural}/{name}` - read
- `PUT    /apis/authorization/v1/{plural}/{name}` - update
- `PATCH  /apis/authorization/v1/{plural}/{name}` - patch
- `DELETE /apis/authorization/v1/{plural}/{name}` - delete

**Role, RoleBinding (namespaced)**:
- `GET    /apis/authorization/v1/{plural}` - list-all
- `GET    /apis/authorization/v1/namespaces/{namespace}/{plural}` - list
- `POST   /apis/authorization/v1/namespaces/{namespace}/{plural}` - create
- `GET    /apis/authorization/v1/namespaces/{namespace}/{plural}/{name}` - read
- `PUT    /apis/authorization/v1/namespaces/{namespace}/{plural}/{name}` - update
- `PATCH  /apis/authorization/v1/namespaces/{namespace}/{plural}/{name}` - patch
- `DELETE /apis/authorization/v1/namespaces/{namespace}/{plural}/{name}` - delete

### resource_registry.rs への登録

- `all_resource_apis()` に4つの `ResourceApiDescriptor` を追加
- 既存の `CLUSTER_DEFAULT_OPS` / `NAMESPACED_DEFAULT_OPS` を利用

### endpoints/mod.rs への登録

- `v1_authorization` モジュールを追加
- `register_endpoints` にrbacルートを追加
- OpenAPIドキュメントに追加

## 確認

```bash
cargo build --package tugboat-apiserver
cargo clippy --package tugboat-apiserver
```

## 参考

- エンドポイントパターン: `tugboat-apiserver/src/endpoints/v1_apps/` の構造を踏襲
- 汎用ハンドラ: `resource_handlers.rs` の `create_cluster<T>`, `create_namespaced<T>` 等を利用
- ルートパターン: 既存のapps/v1やcoordination/v1のルーティングパスを確認 (`/apis/{group}/{version}/...`)
