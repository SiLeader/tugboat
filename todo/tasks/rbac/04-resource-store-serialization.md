# 04: Resource Storeへのシリアライゼーション登録

## 概要

RBACリソースとServiceAccountをetcdに保存できるよう、`tugboat-resource-store`にシリアライゼーションサポートを追加する。

## 作業内容

### tugboat-resource-store/src/serializer/mod.rs

`protobuf_serializable!` マクロを各リソースに適用:

```rust
// rbac.authorization/v1
protobuf_serializable!(Role);
protobuf_serializable!(ClusterRole);
protobuf_serializable!(RoleBinding);
protobuf_serializable!(ClusterRoleBinding);

// core/v1
protobuf_serializable!(ServiceAccount);
```

## 確認

```bash
cargo build --package tugboat-resource-store
cargo test --package tugboat-resource-store
```

## 参考

- 既存パターン: `tugboat-resource-store/src/serializer/mod.rs` の他リソースの登録
