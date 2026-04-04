# Fleet: 共有 NetworkClass の統合

## 概要

`FleetSpec.network_class_name` に指定された `NetworkClass` を、
Fleet 配下の全 Ship に自動的に追加するロジックを実装する。

Fleet 内の全 Ship が同じプライベートネットワークに接続されることで、
相互通信できるようになる。

## 対象ファイル

- `tugboat-controller-manager/src/fleet.rs`

## 実装内容

### ShipTemplate へのネットワーク注入

コンポーネントの `ship_template.spec.network_classes` に Fleet の `network_class_name` を追加してから
ReplicaSet を作成・更新する：

```rust
fn inject_fleet_network(
    template: &mut ShipTemplateSpec,
    network_class_name: &str,
) {
    let spec = template.spec.get_or_insert_with(ShipSpec::default);
    if !spec.network_classes.iter().any(|n| n == network_class_name) {
        spec.network_classes.push(network_class_name.to_string());
    }
}
```

`build_replicaset_for_component` の中で `inject_fleet_network` を呼び出す。

### NetworkClass の存在確認

`Api::<NetworkClass>::namespaced(client, namespace).get(network_class_name)` で存在を確認し、
存在しない場合は `Action::requeue(Duration::from_secs(10))` を返す。

```rust
let nc_api: Api<NetworkClass> = Api::namespaced(self.client.clone(), namespace);
if nc_api.get(&spec.network_class_name).await?.is_none() {
    tracing::warn!(
        "NetworkClass '{}' referenced by Fleet '{}/{}' is not available yet",
        spec.network_class_name, namespace, fleet.name()
    );
    return Ok(Action::requeue(Duration::from_secs(10)));
}
```

### FleetStatus の更新

```rust
let total = spec.components.len() as i32;
let ready = owned_replicasets.iter()
    .filter(|rs| {
        rs.status.as_ref()
            .map(|s| s.ready_replicas == s.replicas && s.replicas > 0)
            .unwrap_or(false)
    })
    .count() as i32;

fleet.status = Some(FleetStatus { ready_components: ready, total_components: total });
fleet_api.replace(fleet.name(), fleet).await?;
```

## 完了条件

- Fleet の全 Ship が `spec.network_class_name` の NetworkClass に接続される
- 指定した NetworkClass が存在しないときは requeue されログが出る
- `FleetStatus.ready_components` / `total_components` が正しく更新される
- `cargo clippy` および `cargo test` が通ること
