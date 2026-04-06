# タスク: RuntimeClass proto 定義の追加

## 概要

`tugboat-resources` に `RuntimeClass` リソースの protobuf 定義を追加し、ビルドシステムに組み込む。

## 実装内容

### 1. proto ファイルの新規作成

`tugboat-resources/proto/core/v1/runtime_class.proto` を以下の内容で作成する:

```protobuf
syntax = "proto3";

import "meta/v1/object_meta.proto";
import "meta/v1/type_meta.proto";

package tugboat.core.v1;

message RuntimeClass {
  meta.v1.TypeMeta type_meta = 1;
  meta.v1.ObjectMeta object_meta = 2;
  RuntimeClassSpec spec = 3;
}

message RuntimeClassSpec {
  bool live_migration = 1;
  RuntimeHotplug hotplug = 2;
}

message RuntimeHotplug {
  HotplugCapabilities cpu = 2;
  HotplugCapabilities memory = 3;
  HotplugCapabilities nic = 4;
  HotplugCapabilities storage = 5;
}

message HotplugCapabilities {
  bool add = 1;
  bool remove = 2;
}
```

### 2. build.rs へのコンパイル対象追加

`tugboat-resources/build.rs` の `compile_protos` 呼び出しに以下を追加する:

```rust
"proto/core/v1/runtime_class.proto",
```

### 3. manifests/mod.rs へのリソース登録

`tugboat-resources/src/manifests/mod.rs` の `core::v1` モジュールに以下を追加する:

```rust
apply_resource!(
    RuntimeClass,
    "core",
    "v1",
    "runtimeclasses",
    "runtimeclass",
    cluster
);
apply_validators!(RuntimeClass, validators NameValidator, NamespaceProhibitedValidator);
```

## 自動テスト

`tugboat-resources/src/manifests/mod.rs` の既存テストモジュール (`core::v1::tests`) に以下を追加する:

```rust
#[test]
fn runtimeclass_with_valid_name_passes_validation() {
    use crate::manifests::meta::v1::ObjectMeta;
    use crate::validators::Validatable;
    let rc = RuntimeClass {
        object_meta: Some(ObjectMeta {
            name: Some("qemu-kvm".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(rc.validate());
}

#[test]
fn runtimeclass_with_invalid_name_fails_validation() {
    use crate::manifests::meta::v1::ObjectMeta;
    use crate::validators::Validatable;
    let rc = RuntimeClass {
        object_meta: Some(ObjectMeta {
            name: Some("Invalid_Name".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!rc.validate());
}

#[test]
fn runtimeclass_with_namespace_fails_validation() {
    use crate::manifests::meta::v1::ObjectMeta;
    use crate::validators::Validatable;
    let rc = RuntimeClass {
        object_meta: Some(ObjectMeta {
            name: Some("qemu-kvm".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!rc.validate());
}
```

## 確認コマンド

```bash
cargo build --package tugboat-resources
cargo test --package tugboat-resources
cargo clippy --package tugboat-resources
```
