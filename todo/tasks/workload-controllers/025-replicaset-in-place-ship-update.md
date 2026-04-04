# ReplicaSet: 既存 Ship のインプレース spec 更新

## 概要

ReplicaSet の `ship_template` が変更されたとき、所有する Ship を削除・再作成するのではなく、
**既存 Ship の `spec` を直接更新**する。エージェントが Ship の spec 変更を検知し、
ホットプラグ・ライブマイグレーション・再作成のいずれが最適かを自動で判断する。

これにより、コントローラー側では Ship を殺さずに済む変更（CPU増設、メモリ追加など）で
不要なダウンタイムを回避できる。

## 背景: エージェントの spec 変更ハンドリング

エージェントの `reconcile_modified`（`tugboat-agent/src/reconciler/ops/modify.rs`）は
3 段階のフィンガープリント比較で変更内容を判別する：

| フィンガープリント | 含まれるフィールド | エージェントの対応 |
|---|---|---|
| `spec` | image, ship_class, network_class_ref, uefi, target_node_name | マイグレーション → 再作成 |
| `pvc_volume` | PVC ボリューム参照 | 再作成 |
| `materialized_volume` | ConfigMap / Secret | インプレースリフレッシュ |

将来ホットプラグが実装されると（`todo/runtimeclass-hotplug.txt`）、`ship_class` 変更時に
CPU/メモリのホットプラグが自動的に使われるようになる。

## 対象ファイル

- `tugboat-controller-manager/src/replicaset.rs`

## 実装内容

### テンプレートと Ship spec の比較

ReplicaSet の `ship_template.spec` と各 Ship の現在の `spec` を比較する。
スケジューリング専用フィールド（`node_name`, `tolerations`, `scheduler_name`）および
`target_node_name` は比較対象から除外する。

```rust
fn needs_spec_update(template_spec: &ShipSpec, ship_spec: &ShipSpec) -> bool {
    // image, ship_class, network_class_ref, uefi, volume_claim_ref, volumes を比較
    // node_name, tolerations, scheduler_name, target_node_name は除外
    template_spec.image != ship_spec.image
        || template_spec.ship_class != ship_spec.ship_class
        || template_spec.network_class_ref != ship_spec.network_class_ref
        || template_spec.uefi != ship_spec.uefi
        || template_spec.volume_claim_ref != ship_spec.volume_claim_ref
        || template_spec.volumes != ship_spec.volumes
}
```

### インプレース更新の適用

テンプレートと Ship が一致しない場合、Ship の spec を更新して API に書き戻す：

```rust
for ship in &owned_ships {
    let ship_spec = ship.spec.as_ref().unwrap();
    if needs_spec_update(&template.spec, ship_spec) {
        let mut updated = ship.clone();
        let updated_spec = updated.spec.as_mut().unwrap();
        // テンプレートの値で上書き（スケジューリングフィールドは維持）
        updated_spec.image = template.spec.image.clone();
        updated_spec.ship_class = template.spec.ship_class.clone();
        updated_spec.network_class_ref = template.spec.network_class_ref.clone();
        updated_spec.uefi = template.spec.uefi.clone();
        updated_spec.volume_claim_ref = template.spec.volume_claim_ref.clone();
        updated_spec.volumes = template.spec.volumes.clone();
        ship_api.replace(ship.name(), updated).await?;
    }
}
```

### 更新の順序

一度にすべての Ship を更新するのではなく、1 台ずつ更新して Ready になるまで待つ
（ローリング方式）。これによりエージェントが再作成を選んだ場合でもサービスの可用性を保てる：

```rust
for ship in &owned_ships {
    if needs_spec_update(&template.spec, ship.spec.as_ref().unwrap()) {
        // spec を更新
        ship_api.replace(ship.name(), updated).await?;
        // 次の reconcile ループで残りを処理する
        return Ok(Action::requeue(Duration::from_secs(5)));
    }
}
```

## 注意事項

- この仕組みはレプリカ数の管理（020, 030）とは独立に動作する
- replicas 変更とテンプレート変更が同時に起きた場合、先にレプリカ数を合わせてから
  インプレース更新を行う

## 完了条件

- ReplicaSet の `ship_template` を変更すると、既存 Ship の spec が更新される
- Ship は 1 台ずつ更新される（ローリング）
- 更新後にエージェントが正しく変更を検知できる（フィンガープリントが変わる）
- `cargo clippy` および `cargo test` が通ること
