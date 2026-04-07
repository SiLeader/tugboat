# TODO: ホットプラグ機能の実装

## 目的

VM実行中に CPU・メモリ・NIC・ストレージデバイスを動的に追加・削除（ホットプラグ）できる機能を `tugboat-runtime` および
`tugboat-vm-runtime-interface` レベルで実装する。

これにより、スペック変更（CPU数・メモリ量の増減、NIC/ストレージの追加削除）が必要な場合に、VMの再起動なしで変更を適用できる。Fleet・Deployment・ReplicaSet
は、対象の RuntimeClass がホットプラグをサポートする場合、ローテーション（VM再作成）ではなくホットプラグを使って実行中の
Ship を更新する。

> **注記**: ホットプラグ機能は過去に実装されていたが、2026-04-02 のコミット `2ab9b3f` で削除された。
> 削除されたファイル:
> - `tugboat-agent/src/reconciler/ops/modify.rs`（ホットプラグロジック部分）
> - `tugboat-agent/src/runtime/hotplug.rs`
> - `tugboat-runtime/src/cmd/hotplug/mod.rs`
> - `tugboat-vm-runtime-interface/src/hotplug.rs`

## 実現する機能

### CPU ホットプラグ

- **ホットアド** (`hotplug.cpu.add`): 実行中の VM に vCPU を追加する
- **ホットリムーブ** (`hotplug.cpu.remove`): 実行中の VM から vCPU を削除する
- QEMU の `device_add` / `cpu-add` QMP コマンドによる実装
- ShipClass の `cpu.cores` 変更をホットプラグで適用

### メモリ ホットプラグ

- **ホットアド** (`hotplug.memory.add`): 実行中の VM にメモリを追加する（DIMM/virtio-mem による追加）
- **ホットリムーブ** (`hotplug.memory.remove`): 実行中の VM からメモリを削除する
- QEMU の `device_add` (pc-dimm / virtio-mem-pci) QMP コマンドによる実装
- ShipClass の `memory.size` 変更をホットプラグで適用

### NIC ホットプラグ

- **ホットアド** (`hotplug.nic.add`): 実行中の VM にネットワークインターフェースを追加する
- **ホットリムーブ** (`hotplug.nic.remove`): 実行中の VM からネットワークインターフェースを削除する
- QEMU の `netdev_add` + `device_add` (virtio-net-pci) QMP コマンドによる実装
- `ShipSpec.network_class_ref` の追加・削除をホットプラグで適用

### ストレージ ホットプラグ

- **ホットアド** (`hotplug.storage.add`): 実行中の VM にブロックデバイス（PVC）を追加する
- **ホットリムーブ** (`hotplug.storage.remove`): 実行中の VM からブロックデバイスを削除する
- QEMU の `blockdev-add` + `device_add` (virtio-blk-pci) QMP コマンドによる実装
- `ShipSpec.volume_claim_ref` の追加・削除をホットプラグで適用

## 仕様

### Runtime Interface への追加（`tugboat-vm-runtime-interface`）

`tugboat-vm-runtime-interface/src/lib.rs` のランタイムトレイトにホットプラグ操作を追加:

```rust
// 追加するオペレーション（案）
async fn hotplug(&self, req: VmHotplugRequest) -> Result<(), RuntimeError>;
```

リクエスト型（案）:

```rust
pub struct VmHotplugCpuRequest {
    pub id: String,
    pub cpu: Option<VmCpuHotplugConfig>,
    pub memory: Option<VmMemoryHotplugConfig>,
    pub nics: Option<VmNicHotplugConfig>,
    pub volumes: Option<VmHotplugVolumeRequest>,
}

pub struct VmCpuHotplugConfig {
    pub cores: u64,          // 変更後の合計 vCPU 数
}

pub struct VmMemoryHotplugConfig {
    pub size: String,        // 変更後の合計メモリ量（例: "4Gi"）
}

pub struct VmNicHotplugConfig {
    pub added: VmNetworkConfig,  // 既存の VmNetworkConfig を流用
    pub removed: VmNetworkConfig,
}

pub struct VmHotplugVolumeRequest {
    pub vm_id: String,
    pub added: VmVolumeConfig,   // 既存の VmVolumeConfig を流用
    pub removed: VmVolumeConfig,
}
```

### Ship Status への実際のリソース割り当て追加

`tugboat-resources/proto/core/v1/ship.proto` の `ShipStatus` に実際の割り当て状態を追加:

```protobuf
message ShipStatus {
  // ...既存フィールド...
  optional ShipActualAllocation actual_allocation = 4;
}

message ShipActualAllocation {
  optional uint64 cpu_cores = 1;       // 実際に割り当て済みの vCPU 数
  optional string memory_size = 2;     // 実際に割り当て済みのメモリ量
  repeated string hotplugged_nic_ids = 3;
  repeated string hotplugged_volume_ids = 4;
}
```

### Agent での更新フロー（ホットプラグパス）

`tugboat-agent/src/reconciler/ops/modify.rs` にホットプラグの判定・実行ロジックを追加:

```
Ship spec 変更検出
  │
  ├─ ShipClass変更（CPUコア数増）かつ RuntimeClass.hotplug.cpu.add == true
  │     └─→ hotplug を呼び出し
  │
  ├─ ShipClass変更（CPUコア数減）かつ RuntimeClass.hotplug.cpu.remove == true
  │     └─→ hotplug を呼び出し（非対応なら rotate）
  │
  ├─ ShipClass変更（メモリ増）かつ RuntimeClass.hotplug.memory.add == true
  │     └─→ hotplug を呼び出し
  │
  ├─ ShipClass変更（メモリ減）かつ RuntimeClass.hotplug.memory.remove == true
  │     └─→ hotplug を呼び出し（非対応なら rotate）
  │
  ├─ network_class_ref 追加かつ RuntimeClass.hotplug.nic.add == true
  │     └─→ hotplug を呼び出し
  │
  ├─ network_class_ref 削除かつ RuntimeClass.hotplug.nic.remove == true
  │     └─→ hotplug を呼び出し
  │
  ├─ volume_claim_ref 追加かつ RuntimeClass.hotplug.storage.add == true
  │     └─→ hotplug を呼び出し
  │
  ├─ volume_claim_ref 削除かつ RuntimeClass.hotplug.storage.remove == true
  │     └─→ hotplug を呼び出し
  │
  └─ ホットプラグ非対応またはホットプラグ不可の変更
        └─→ 既存の recreate / rotate パス
```

同時に複数のホットプラグが行われる場合は、すべてを一つのランタイムリクエストとしてランタイムを呼び出す。
非対応のオペレーションと対応済みオペレーションの両方が存在する場合は、非対応のオペレーションは設定変更を行わないのと同じように扱う。

### Controller Manager での変更分類

`tugboat-controller-manager/src/change_classifier.rs` の `TemplateChangeKind` を拡張:

```rust
pub(crate) enum TemplateChangeKind {
    NoChange,
    InPlace,        // 既存: 再起動不要な変更
    Hotplug,        // 新規: ホットプラグで適用可能な変更
    RequiresRotation, // 既存: VM 再作成が必要な変更
}
```

ホットプラグ可能な変更の判定には、対象 Ship が参照する RuntimeClass のフラグを参照する。

## 影響範囲

| コンポーネント                        | 変更内容                                                                       |
|--------------------------------|----------------------------------------------------------------------------|
| `tugboat-vm-runtime-interface` | ホットプラグ操作のトレイト定義追加                                                          |
| `tugboat-runtime`              | QEMU QMP 経由のホットプラグコマンド実装                                                   |
| `tugboat-agent`                | ホットプラグ判定ロジックと実行パスの追加                                                       |
| `tugboat-resources`            | `ShipStatus` への `actual_allocation` フィールド追加                                |
| `tugboat-controller-manager`   | `change_classifier.rs` の変更種別拡張、Fleet/Deployment/ReplicaSet コントローラのホットプラグ対応 |
| `tugboat-apiserver`            | ホットプラグ操作の Ship ステータス更新 API（`/status` エンドポイント経由）                            |

## 実装方針

1. **Runtime Interface 定義**: `tugboat-vm-runtime-interface/src/hotplug.rs` を新規作成し、ホットプラグオペレーションのトレイトを定義
2. **QEMU 実装**: `tugboat-runtime/src/cmd/hotplug/` 配下にコマンド実装を追加（QMP の `device_add`、`device_del`、
   `netdev_add`、`blockdev-add` を使用）
3. **Agent の reconcile 拡張**: `tugboat-agent/src/reconciler/ops/modify.rs` にホットプラグパスを追加。RuntimeClass
   を参照して実行可否を判定してからホットプラグを試みる
4. **変更分類の拡張**: `change_classifier.rs` に `Hotplug` 分類を追加し、Fleet/Deployment コントローラがホットプラグ可能な変更を識別できるようにする
5. **Ship ステータス更新**: ホットプラグ成功後、Agent が `ShipStatus.actual_allocation` を更新する
6. **ReplicaSet コントローラ**: `Hotplug` 分類の変更が検出された場合、Ships を直接ホットプラグパスで更新する（再作成なし）

### ホットプラグと既存機能の相互作用

- **ライブマイグレーション中**: ホットプラグ操作はライブマイグレーション中はスキップし（`ShipMigrationStatus`
  を確認）、マイグレーション完了後に再試行する
- **ホットプラグ失敗時**: エラーをログに記録し、Ship の condition に反映した上でローテーション（VM 再作成）にフォールバックする

## 受け入れ条件

### CPU ホットプラグ

- [ ] ShipClass の `cpu.cores` を増やしたとき、RuntimeClass の `hotplug.cpu.add` が `true` のノード上の Ship が再起動なしに
  vCPU を追加できる
- [ ] ShipClass の `cpu.cores` を減らしたとき、RuntimeClass の `hotplug.cpu.remove` が `true` のノード上の Ship が再起動なしに
  vCPU を削除できる
- [ ] ゲスト OS から `nproc` 等で変更後の vCPU 数が確認できる

### メモリ ホットプラグ

- [ ] ShipClass の `memory.size` を増やしたとき、RuntimeClass の `hotplug.memory.add` が `true` のノード上の Ship
  が再起動なしにメモリを拡張できる
- [ ] ShipClass の `memory.size` を減らしたとき、RuntimeClass の `hotplug.memory.remove` が `true` のノード上の Ship
  が再起動なしにメモリを削減できる
- [ ] ゲスト OS から `free -m` 等で変更後のメモリ量が確認できる

### NIC ホットプラグ

- [ ] `ShipSpec.network_class_ref` に NetworkClass を追加したとき、RuntimeClass の `hotplug.nic.add` が `true` のノード上の
  Ship が再起動なしに NIC を追加できる
- [ ] `ShipSpec.network_class_ref` から NetworkClass を削除したとき、RuntimeClass の `hotplug.nic.remove` が `true`
  のノード上の Ship が再起動なしに NIC を削除できる

### ストレージ ホットプラグ

- [ ] `ShipSpec.volume_claim_ref` に PVC を追加したとき、RuntimeClass の `hotplug.storage.add` が `true` のノード上の
  Ship が再起動なしにブロックデバイスを追加できる
- [ ] `ShipSpec.volume_claim_ref` から PVC を削除したとき、RuntimeClass の `hotplug.storage.remove` が `true` のノード上の
  Ship が再起動なしにブロックデバイスを削除できる

### コントローラ連携

- [ ] Fleet/Deployment のテンプレート変更時、ホットプラグ可能な変更は新しい ReplicaSet を作らずに既存 Ships をホットプラグで更新する
- [ ] RuntimeClass の当該ホットプラグフラグ（`add` または `remove`）が `false` の場合はローテーションにフォールバックする
- [ ] `cargo test` および `cargo clippy` がエラーなく通過する

## 備考

- **前提**: RuntimeClass リソース（`todo/runtimeclass.md` 参照）が実装済みであること。ホットプラグの実行可否は RuntimeClass
  のフラグに基づいて判断する
- **QEMU バージョン**: ホットプラグ機能は QEMU の machine type `q35` および KVM が有効な環境での動作を前提とする（現在の実装と同様）
- **ゲスト OS サポート**: CPU・メモリのホットプラグはゲスト OS の ACPI 対応が必要。NIC・ストレージは virtio ドライバが必要
- **メモリのホットリムーブ**: メモリの動的削除（ホットリムーブ）はゲスト OS のメモリバルーニング対応が必要で技術的に複雑。初期実装では
  `hotplug.memory.remove = false` として削除は再起動（ローテーション）で対応し、後から `true` にすることを検討する
- **実装履歴**: 以前の実装は `tugboat-vm-runtime-interface/src/hotplug.rs` 等に存在していたが、コミット `2ab9b3f`
  （2026-04-02）で削除された。再実装の際は削除の経緯を確認すること
