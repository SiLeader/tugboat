# Fleet リソースの proto 定義

## 概要

Fleet は複数種類の Ship を一つのグループとして管理し、各 Ship が同一のプライベートネットワーク
（`NetworkClass`）を共有するリソースである。Kubernetes の StatefulSet に近いが、
VM の種類が複数混在できる点が特徴。

`tugboat-resources/proto/apps/v1/fleet.proto` を新規作成し、Fleet の protobuf メッセージを定義する。

## 対象ファイル

- `tugboat-resources/proto/apps/v1/fleet.proto`（新規作成）
- `tugboat-resources/build.rs`（コンパイル対象への追加）

## proto の設計

```protobuf
syntax = "proto3";

import "meta/v1/object_meta.proto";
import "meta/v1/type_meta.proto";
import "apps/v1/workload.proto";

package tugboat.apps.v1;

message Fleet {
  tugboat.meta.v1.TypeMeta type_meta = 1;
  tugboat.meta.v1.ObjectMeta object_meta = 2;
  FleetSpec spec = 3;
  optional FleetStatus status = 4;
}

message FleetSpec {
  // 共有する NetworkClass 名
  string network_class_name = 1;
  // 構成する Ship の種類ごとの定義
  repeated FleetComponent components = 2;
}

message FleetComponent {
  // コンポーネント名（例: "master", "worker"）
  string name = 1;
  // このコンポーネントの Ship 数
  int32 replicas = 2;
  // Ship テンプレート
  ShipTemplateSpec ship_template = 3;
}

message FleetStatus {
  int32 ready_components = 1;
  int32 total_components = 2;
}
```

## build.rs への追加

```rust
.compile_protos(
    &[
        // 既存のエントリ...
        "proto/apps/v1/fleet.proto",
    ],
    // ...
)?;
```

## 完了条件

- `cargo build --release --package tugboat-resources` が通ること
- `Fleet`、`FleetSpec`、`FleetComponent`、`FleetStatus` 型が生成される
