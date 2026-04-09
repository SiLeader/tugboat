# 01: RBAC Protobuf定義

## 概要

`rbac.authorization/v1` APIグループのProtobufメッセージを定義する。

## 対象リソース

### Role (namespaced)

```protobuf
message Role {
  TypeMeta type_meta = 1;
  ObjectMeta object_meta = 2;
  repeated PolicyRule rules = 3;
}
```

### ClusterRole (cluster-scoped)

```protobuf
message ClusterRole {
  TypeMeta type_meta = 1;
  ObjectMeta object_meta = 2;
  repeated PolicyRule rules = 3;
  repeated AggregationRule aggregation_rule = 4; // optional
}
```

### RoleBinding (namespaced)

```protobuf
message RoleBinding {
  TypeMeta type_meta = 1;
  ObjectMeta object_meta = 2;
  repeated Subject subjects = 3;
  RoleRef role_ref = 4;
}
```

### ClusterRoleBinding (cluster-scoped)

```protobuf
message ClusterRoleBinding {
  TypeMeta type_meta = 1;
  ObjectMeta object_meta = 2;
  repeated Subject subjects = 3;
  RoleRef role_ref = 4;
}
```

### 共通メッセージ

```protobuf
message PolicyRule {
  repeated string api_groups = 1;
  repeated string resources = 2;
  repeated string resource_names = 3;
  repeated string verbs = 4;
  // repeated string non_resource_urls = 5; // 必要に応じて追加
}

message Subject {
  string kind = 1;       // "User", "Group", "ServiceAccount"
  string api_group = 2;  // "rbac.authorization" or ""(core)
  string name = 3;
  string namespace = 4;  // ServiceAccountの場合のみ
}

message RoleRef {
  string api_group = 1;  // "rbac.authorization"
  string kind = 2;       // "Role" or "ClusterRole"
  string name = 3;
}
```

## 作業内容

1. `tugboat-resources/proto/rbac_authorization/v1/` ディレクトリを作成
2. 以下のprotoファイルを作成:
   - `policy_rule.proto` (PolicyRule, Subject, RoleRef)
   - `role.proto` (Role)
   - `cluster_role.proto` (ClusterRole)
   - `role_binding.proto` (RoleBinding)
   - `cluster_role_binding.proto` (ClusterRoleBinding)
3. `tugboat-resources/build.rs` に新しいprotoファイルを追加
4. コンパイルが通ることを確認 (`cargo build --package tugboat-resources`)

## 参考

- 既存のproto定義: `tugboat-resources/proto/core/v1/`, `tugboat-resources/proto/apps/v1/`
- build.rs: serde属性やcamelCase変換の適用パターンを踏襲する
- Kubernetes対応: `rbac.authorization.k8s.io/v1` → `rbac.authorization/v1`
