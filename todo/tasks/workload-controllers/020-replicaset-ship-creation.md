# ReplicaSet: 不足 Ship の作成

## 概要

`ReplicaSetReconciler::reconcile_applied` に、desired replicas に対して Ship が不足している場合に
Ship を作成するロジックを実装する。

## 対象ファイル

- `tugboat-controller-manager/src/replicaset.rs`

## 実装内容

### reconcile_applied の基本フロー

1. `spec.replicas` を取得（`None` のときはデフォルト 1 とみなす）
2. `Api::<Ship>::namespaced(client, namespace)` で全 Ship をリストし、セレクターに一致するものを所有 Ship とする
3. `owned_ships.len() < desired` なら不足分だけ Ship を作成する

### Ship の生成

`ShipTemplateSpec` から Ship を組み立てる：

```rust
fn build_ship(rs: &ReplicaSet, template: &ShipTemplateSpec, index: usize) -> Ship {
    let name = format!("{}-{}", rs.name(), generate_suffix(index));
    let mut meta = template.metadata.clone();
    meta.name = Some(name);
    meta.namespace = rs.namespace().map(str::to_string);
    // セレクターラベルをマージ
    meta.labels.extend(rs.spec.selector.clone());
    // ownerReference を付与
    meta.owner_references.push(owner_reference_for_replicaset(rs));

    Ship {
        type_meta: set_type_meta(),
        object_meta: meta,
        spec: Some(template.spec.clone()),
        status: None,
    }
}
```

サフィックスはランダムな英数字 5 文字（例: `rand::distributions::Alphanumeric`）を使う。

### 作成 API 呼び出し

```rust
let ship_api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
ship_api.create(ship).await?;
```

409 Conflict は冪等として無視する。

## 完了条件

- desired より少ない Ship しかないとき、差分だけ Ship が作成される
- 作成された Ship の labels にセレクターラベルが含まれる
- ownerReference に ReplicaSet が設定されている
- `cargo test` が通ること
