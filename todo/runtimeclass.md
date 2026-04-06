# TODO: RuntimeClass リソースの導入

## 目的

VMランタイムの実装が持つケーパビリティを宣言するクラスタスコープのリソース `RuntimeClass` を `core/v1` に追加する。

現在 `ShipClass` はVMのスペック（CPU数、メモリサイズ、ライブマイグレーションのチューニングパラメータ）を定義しているが、*
*「そのノードのランタイムが何をサポートするか」** を表すリソースは存在しない。RuntimeClass
はこのケーパビリティ情報を保持し、スケジューラや各コントローラがランタイムの機能を考慮した意思決定を行えるようにする。

## 実現する機能

| フィールド                    | 型      | 説明                         |
|--------------------------|--------|----------------------------|
| `live_migration`         | `bool` | ライブマイグレーションの対応可否           |
| `hotplug.cpu.add`        | `bool` | CPU ホットアドの対応可否             |
| `hotplug.cpu.remove`     | `bool` | CPU ホットリムーブの対応可否           |
| `hotplug.memory.add`     | `bool` | メモリ ホットアドの対応可否             |
| `hotplug.memory.remove`  | `bool` | メモリ ホットリムーブの対応可否           |
| `hotplug.nic.add`        | `bool` | NIC ホットアドの対応可否             |
| `hotplug.nic.remove`     | `bool` | NIC ホットリムーブの対応可否           |
| `hotplug.storage.add`    | `bool` | ストレージ ホットアドの対応可否           |
| `hotplug.storage.remove` | `bool` | ストレージ ホットリムーブの対応可否         |

## 仕様

### Proto 定義（案）

`tugboat-resources/proto/core/v1/runtime_class.proto` に新規追加:

```protobuf
syntax = "proto3";
package tugboat.core.v1;

import "meta/v1/meta.proto";

message RuntimeClass {
  meta.v1.TypeMeta type_meta = 1;
  meta.v1.ObjectMeta object_meta = 2;
  RuntimeClassSpec spec = 3;
}

message RuntimeClassSpec {
  // ライブマイグレーション対応
  bool live_migration = 1;
  // ホットプラグ対応
  RuntimeHotplug hotplug = 2;
}

message RuntimeHotplug {
  // CPU ホットプラグ対応
  HotplugCapabilities cpu = 2;
  // メモリ ホットプラグ対応
  HotplugCapabilities memory = 3;
  // NIC ホットプラグ対応
  HotplugCapabilities nic = 4;
  // ストレージ ホットプラグ対応
  HotplugCapabilities storage = 5;
}

message HotplugCapabilities {
  bool add = 1;
  bool remove = 2;
}
```

### Ship との関連付け

`tugboat-resources/proto/core/v1/ship.proto` の `ShipSpec` に `runtime_class` フィールドを追加することで、
ShipがどのRuntimeClassを使用するかを指定できるようにする:

```protobuf
message ShipSpec {
  // ...既存フィールド...
  optional string runtime_class = 11; // 参照する RuntimeClass 名
}
```

### リソース特性

- **スコープ**: クラスタスコープ（非 Namespace）
- **group/version/kind**: `core/v1/RuntimeClass`
- **plural/singular**: `runtimeclasses` / `runtimeclass`
- **バリデータ**: `NameValidator`

## 影響範囲

| コンポーネント                      | 変更内容                                                        |
|------------------------------|-------------------------------------------------------------|
| `tugboat-resources`          | proto 定義追加、`apply_resource!` / `apply_validators!` 登録       |
| `tugboat-apiserver`          | CRUD エンドポイント追加（クラスタスコープのため `/v1/runtimeclasses` パターン）       |
| `tugboat-scheduler`          | Ship スケジューリング時にノードの RuntimeClass を参照して対応可否を確認               |
| `tugboat-controller-manager` | `change_classifier.rs` でホットプラグ可能な変更を識別する際に RuntimeClass を参照 |
| `tugboat-agent`              | Ship の `runtime_class` フィールドを読み取り、ホットプラグやマイグレーション操作の可否を判断   |

## 実装方針

1. **proto 定義**: `tugboat-resources/proto/core/v1/runtime_class.proto` を新規作成し、`build.rs` のコンパイルリストに追加
2. **リソース登録**: `tugboat-resources/src/manifests/mod.rs` に `apply_resource!` と `apply_validators!` を追加
3. **APIエンドポイント**: `tugboat-apiserver/src/endpoints/v1_core/` にクラスタスコープリソースとして `runtime_class.rs`
   を作成（`cluster_resources.rs` の汎用ハンドラを活用）
4. **エンドポイント登録**: `endpoints/v1_core/mod.rs` にルートを登録
5. **スケジューラ連携**: Ship の `runtime_class` フィールドが指定されている場合、スケジューラがそのノードで対応する
   RuntimeClass が存在するかを確認
6. **Ship proto 更新**: `ShipSpec` に `runtime_class` フィールドを追加

既存の実装パターンとして `ShipClass` (`tugboat-resources/proto/core/v1/ship_class.proto`) および対応するエンドポイントが参考になる。

## 受け入れ条件

- [ ] `RuntimeClass` リソースを `kubectl`（tugboat-cli）で CRUD できる
- [ ] `live_migration` および `hotplug.{cpu,memory,nic,storage}.{add,remove}` の各フラグを設定・取得できる
- [ ] Ship の `spec.runtimeClass` に RuntimeClass 名を指定できる
- [ ] スケジューラが RuntimeClass の `live_migration` フラグを参照し、ライブマイグレーション非対応のノードには対応が必要な
  Ship をスケジュールしない
- [ ] API サーバーの OpenAPI スキーマに RuntimeClass が含まれる
- [ ] `cargo test` および `cargo clippy` がエラーなく通過する

## 備考

- **ShipClass との違い**: ShipClass は Ship インスタンスのスペック（CPU数・メモリ量）を定義する。RuntimeClass
  はランタイム実装のケーパビリティを定義する。1つのノードが複数の RuntimeClass に対応することは想定しない（ノードに
  RuntimeClass を1対1で対応させる設計）
- **ライブマイグレーション**: 現在ライブマイグレーションは実装済み（`tugboat-agent/src/reconciler/ops/migration.rs`
  ）だが、RuntimeClass の `live_migration` フラグによるスケジューリング制御は未実装
- **ホットプラグとの関係**: RuntimeClass の各ホットプラグフラグは、ホットプラグ機能（`todo/hotplug.md`
  参照）の前提条件となる。コントローラはホットプラグ操作の前に RuntimeClass を参照して可否を確認する
