# Fleet: コンポーネントごとの Ship 管理

## 概要

`FleetSpec.components` の各 `FleetComponent` に対して、
`replicas` 分の Ship を作成・削除・維持するロジックを実装する。
各コンポーネントは独立した ReplicaSet に対応させる（Fleet が ReplicaSet を所有する）。

## 対象ファイル

- `tugboat-controller-manager/src/fleet.rs`

## 実装内容

### reconcile_applied の基本フロー

1. Fleet が所有する全 ReplicaSet をリストする
2. 各 `FleetComponent` に対して対応する ReplicaSet を特定する（`fleet-component` ラベルで識別）
3. ReplicaSet が存在しなければ作成する
4. ReplicaSet の `spec.replicas` が `component.replicas` と異なれば更新する
5. **テンプレート変更がある場合**: ReplicaSet の `ship_template` を直接更新する（インプレースパス）。
   ReplicaSet コントローラーが Ship を 1 台ずつ更新し、エージェントがホットプラグや
   マイグレーションで対応する
6. `components` に存在しなくなったコンポーネントの ReplicaSet は削除する

### コンポーネント識別ラベル

- `fleet-name: <fleet-name>`
- `fleet-component: <component-name>`

### ReplicaSet 生成

```rust
fn build_replicaset_for_component(
    fleet: &Fleet,
    component: &FleetComponent,
    network_class_name: &str,
) -> ReplicaSet {
    let name = format!("{}-{}", fleet.name(), component.name);
    let selector = HashMap::from([
        ("fleet-name".to_string(), fleet.name().to_string()),
        ("fleet-component".to_string(), component.name.clone()),
    ]);

    // Ship テンプレートに Fleet のネットワークを注入（150 タスクで詳細を実装）
    let mut template = component.ship_template.clone();
    inject_fleet_network(&mut template, network_class_name);

    ReplicaSet {
        spec: ReplicaSetSpec {
            replicas: Some(component.replicas),
            selector,
            ship_template: template,
        },
        // ownerReference: Fleet
        ..
    }
}
```

### テンプレート更新（インプレースパス）

コンポーネントの `ship_template` が変更された場合、既存 ReplicaSet のテンプレートを
直接更新する。Deployment と異なりローテーションパスは使わない
（Fleet は多種多様な Ship を束ねるため、RS ローテーションは複雑になりすぎる）：

```rust
for component in &spec.components {
    if let Some(existing_rs) = find_rs_for_component(&owned_replicasets, &component.name) {
        let mut needs_update = false;
        let mut updated_rs = existing_rs.clone();

        // レプリカ数の変更
        if existing_rs.spec.replicas != Some(component.replicas) {
            updated_rs.spec.replicas = Some(component.replicas);
            needs_update = true;
        }

        // テンプレートの変更（ネットワーク注入済みで比較）
        let mut desired_template = component.ship_template.clone();
        inject_fleet_network(&mut desired_template, &spec.network_class_name);
        if existing_rs.spec.ship_template != desired_template {
            updated_rs.spec.ship_template = desired_template;
            needs_update = true;
        }

        if needs_update {
            rs_api.replace(existing_rs.name(), updated_rs).await?;
        }
    } else {
        // 新規 RS を作成
        let rs = build_replicaset_for_component(fleet, component, &spec.network_class_name);
        rs_api.create(rs).await?;
    }
}
```

### reconcile_deleted

Fleet 削除時は所有する全 ReplicaSet を削除する（ReplicaSet コントローラーが Ship を削除する）。

## 完了条件

- Fleet を作成すると各コンポーネントに対応する ReplicaSet が作成される
- コンポーネントの `ship_template` を変更すると ReplicaSet のテンプレートが直接更新される
- `components` のエントリを削除・追加すると ReplicaSet が追従する
- `cargo clippy` が通ること
