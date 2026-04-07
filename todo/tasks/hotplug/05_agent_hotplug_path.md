# タスク 05: Agent reconcile_modified へのホットプラグパス追加

## 前提タスク

- タスク 01 (Runtime Interface 型定義) が完了していること
- タスク 02 (ShipStatus actual_allocation) が完了していること
- タスク 04 (VmRuntimeOperator.hotplug) が完了していること
- RuntimeClass リソース (`todo/runtimeclass.md`) が実装済みであること

## 目的

`tugboat-agent/src/reconciler/ops/modify.rs` に、Ship spec 変更を検出してホットプラグを試みる
ロジックを追加する。現在すべての spec 変更は `reconcile_recreate` に流れているが、
RuntimeClass のホットプラグフラグが有効な場合はホットプラグを優先する。

## 実装内容

### `tugboat-agent/src/reconciler/ops/modify.rs` の変更

`reconcile_modified` の spec_changed ブランチに、`reconcile_recreate` を呼ぶ前にホットプラグ判定を挟む。

#### ホットプラグ判定ロジック

1. Ship の `spec.ship_class` から ShipClass を取得し、CPU コア数・メモリサイズを確認
2. Ship の `spec.node_name` のノードから RuntimeClass を取得し、ホットプラグフラグを確認
3. 変更内容とフラグを照合して、ホットプラグ可能な変更セットと不可能な変更セットに分類

#### 変更分類の判定

```
現在のActualAllocation（または起動時スペック）との差分を計算:
  - cpu.cores が変化: RuntimeClass.hotplug.cpu.add / .remove フラグを確認
  - memory.size が変化: RuntimeClass.hotplug.memory.add / .remove フラグを確認
  - network_class_ref に追加あり: RuntimeClass.hotplug.nic.add フラグを確認
  - network_class_ref から削除あり: RuntimeClass.hotplug.nic.remove フラグを確認
  - volume_claim_ref に追加あり: RuntimeClass.hotplug.storage.add フラグを確認
  - volume_claim_ref から削除あり: RuntimeClass.hotplug.storage.remove フラグを確認
```

- ホットプラグ非対応の変更が1つでも存在する場合、その変更は spec 更新なしで無視（todo に記述の通り）
- ホットプラグ対応の変更はすべて1つの `VmHotplugRequest` にまとめて `runtime_operator.hotplug()` を呼び出す
- ライブマイグレーション中（`ShipMigrationStatus` が Pending/Ready/Migrating）はホットプラグをスキップし
  マイグレーション完了後の次の reconcile に委ねる

#### ホットプラグ成功後の処理

- `ShipStatus.actual_allocation` を更新して API サーバーに PATCH する
- `ShipCondition` に `Hotplugged` ステータスを追記する

#### ホットプラグ失敗時の処理

- エラーをログに記録する（`warn!` レベル）
- `ShipCondition` に `HotplugFailed` ステータスを追記する
- `reconcile_recreate` にフォールバックする

### `tugboat-agent/src/reconciler/ops/` への新規ファイル追加（推奨）

ホットプラグ判定ロジックが複雑になる場合は、`hotplug.rs` として分離することを推奨する
（`modify.rs` の肥大化を避けるため）。

```
tugboat-agent/src/reconciler/ops/hotplug.rs
  - classify_hotplug_changes(old_spec, new_spec, actual_alloc, runtime_class) -> HotplugPlan
  - struct HotplugPlan { hotplug_req: Option<VmHotplugRequest>, has_unsupported_changes: bool }
```

## テスト

`hotplug.rs` に以下の単体テストを追加:

```rust
#[test]
fn cpu_increase_with_flag_enabled_is_hotpluggable() {
    // RuntimeClass.hotplug.cpu.add = true のとき classify の結果に cpu が含まれること
}

#[test]
fn cpu_increase_with_flag_disabled_returns_empty_plan() {
    // RuntimeClass.hotplug.cpu.add = false のとき hotplug_req が None になること
}

#[test]
fn mixed_hotplug_and_unsupported_change() {
    // cpu 変更(対応)+ image 変更(非対応)がある場合、cpu だけホットプラグ対象になること
}
```
