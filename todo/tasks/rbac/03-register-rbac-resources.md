# 03: RBACリソースの登録 (tugboat-resources)

## 概要

01・02で定義したProtobufメッセージに対して、リソーストレイト実装とバリデーターを適用する。

## 作業内容

### tugboat-resources/src/manifests/mod.rs

1. `authorization/v1` モジュールのincludeを追加
2. 以下のリソースに `apply_resource!` マクロを適用:

```rust
// authorization/v1 - cluster-scoped
apply_resource!(ClusterRole, "authorization", "v1", "clusterroles", "clusterrole", cluster);
apply_resource!(ClusterRoleBinding, "authorization", "v1", "clusterrolebindings", "clusterrolebinding", cluster);

// authorization/v1 - namespaced
apply_resource!(Role, "authorization", "v1", "roles", "role", namespaced);
apply_resource!(RoleBinding, "authorization", "v1", "rolebindings", "rolebinding", namespaced);
```

3. 以下のリソースに `apply_validators!` マクロを適用:

```rust
apply_validators!(ClusterRole, validators NameValidator, NamespaceProhibitedValidator);
apply_validators!(ClusterRoleBinding, validators NameValidator, NamespaceProhibitedValidator);
apply_validators!(Role, validators NameValidator);
apply_validators!(RoleBinding, validators NameValidator);
```

4. ServiceAccountについても同様:

```rust
apply_resource!(ServiceAccount, "core", "v1", "serviceaccounts", "serviceaccount", namespaced);
apply_validators!(ServiceAccount, validators NameValidator);
```

### tugboat-resources/src/lib.rs

- 必要に応じて新しいモジュールのpub useを追加

## 確認

```bash
cargo build --package tugboat-resources
cargo test --package tugboat-resources
```

## 参考

- 既存パターン: `tugboat-resources/src/manifests/mod.rs` のcore/v1, apps/v1の登録部分
