# ReplicaSet のラベルセレクターによる Ship 所有判定

## 概要

`ReplicaSetSpec.selector`（`map<string, string>`）を使って、
ReplicaSet が「所有する」Ship を特定するロジックを実装する。
Ship の `labels` がセレクターのすべてのキー・バリューを含む場合に所有対象とみなす。

また、ReplicaSet が作成した Ship には `owner_references` に ReplicaSet への参照を付与する。
これにより、次のタスク（Ship 作成・削除）で正しい Ship を対象にできるようになる。

## 対象ファイル

- `tugboat-controller-manager/src/replicaset.rs`

## 実装内容

### ラベルセレクター判定ヘルパー

```rust
fn matches_selector(labels: &std::collections::HashMap<String, String>, selector: &std::collections::HashMap<String, String>) -> bool {
    selector.iter().all(|(k, v)| labels.get(k).map(|lv| lv == v).unwrap_or(false))
}
```

### セレクターに一致する Ship のフィルタリング

`Api::<Ship>::namespaced(client, namespace)` で Ship をリストし、
`ship.labels()` と `rs.spec.selector` を照合して所有 Ship を絞り込む。

### ownerReference ヘルパー

```rust
fn owner_reference_for_replicaset(rs: &ReplicaSet) -> OwnerReference {
    OwnerReference {
        api_version: "apps/v1".to_string(),
        kind: "ReplicaSet".to_string(),
        name: rs.name().to_string(),
        uid: rs.uid().unwrap_or_default().to_string(),
        controller: Some(true),
    }
}
```

Ship 作成時にこの ownerReference を `object_meta.owner_references` に設定する。

## 完了条件

- `matches_selector` と `owner_reference_for_replicaset` ヘルパーが `replicaset.rs` に実装されている
- `cargo clippy` で警告がないこと
