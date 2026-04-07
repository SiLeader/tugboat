# タスク 06: change_classifier への Hotplug 分類追加

## 前提タスク

- RuntimeClass リソース (`todo/runtimeclass.md`) が実装済みであること

## 目的

`tugboat-controller-manager/src/change_classifier.rs` の `TemplateChangeKind` に `Hotplug` を追加し、
Fleet・Deployment コントローラがホットプラグ可能な変更を識別できるようにする。

## 実装内容

### `tugboat-controller-manager/src/change_classifier.rs` の変更

#### `TemplateChangeKind` 列挙型の拡張

```rust
pub(crate) enum TemplateChangeKind {
    NoChange,
    InPlace,
    Hotplug,          // 新規追加: ホットプラグで適用可能な変更
    RequiresRotation,
}
```

#### `classify_template_change` 関数のシグネチャ変更

ホットプラグ可否の判定に RuntimeClass のフラグが必要なため、関数シグネチャを拡張する:

```rust
pub(crate) fn classify_template_change(
    old: &ShipTemplateSpec,
    new: &ShipTemplateSpec,
    runtime_class: Option<&RuntimeClass>,  // RuntimeClass が取得できない場合は None
) -> TemplateChangeKind
```

RuntimeClass が `None` の場合は従来通り `InPlace` / `RequiresRotation` で判定する。

#### 判定ロジック

1. まず `RequiresRotation` の判定を行う（image、uefi の変更）
2. `RequiresRotation` でなければ、`network_class_ref` や `volume_claim_ref` の変更が
   RuntimeClass のホットプラグフラグですべてカバーできるか確認する
3. すべてカバーできれば `Hotplug`、一部でもカバーできなければ `RequiresRotation`
4. NIC/ストレージ以外の変更（ship_class の変更など）が RuntimeClass のフラグでカバーされる場合も `Hotplug`
5. 変更がなければ `NoChange`、ホットプラグ不要な変更のみなら `InPlace`

注意: 現在 `classify_template_change` を呼び出している箇所（fleet.rs, replicaset.rs 等）は
`runtime_class: None` を渡すことで従来の動作を維持しつつ、後続タスクで RuntimeClass 取得ロジックを追加できる。

## テスト

既存テストが `runtime_class: None` を渡すよう更新し、引き続き通過することを確認する。
新規テストを以下の観点で追加:

```rust
#[test]
fn ship_class_change_is_hotplug_when_cpu_add_supported() {
    // RuntimeClass.hotplug.cpu.add = true かつ ship_class 変更（CPU増）→ Hotplug
}

#[test]
fn nic_add_is_hotplug_when_flag_enabled() {
    // RuntimeClass.hotplug.nic.add = true かつ network_class_ref 追加 → Hotplug
}

#[test]
fn nic_add_requires_rotation_when_flag_disabled() {
    // RuntimeClass.hotplug.nic.add = false → RequiresRotation
}

#[test]
fn mixed_hotplug_and_non_hotplug_requires_rotation() {
    // image 変更 + NIC 追加(hotplug対応) → RequiresRotation（imageはホットプラグ不可）
}
```
