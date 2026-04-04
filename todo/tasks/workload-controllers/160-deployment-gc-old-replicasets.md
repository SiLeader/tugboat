# Deployment: 不要になった ReplicaSet のガベージコレクション

## 概要

Deployment が削除された場合、または古い ReplicaSet の `replicas` が 0 になってから
一定時間が経過した場合に、不要な ReplicaSet を削除する。

## 対象ファイル

- `tugboat-controller-manager/src/deployment.rs`

## 実装内容

### Deployment 削除時（reconcile_deleted）

Deployment の `reconcile_deleted` で所有する全 ReplicaSet を削除する：

```rust
async fn reconcile_deleted(&self, dep: Deployment) -> Result<Action, ControllerError> {
    let namespace = dep.namespace().unwrap_or_default();
    let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), &namespace);
    let rsets = rs_api.list().await?;
    for rs in rsets.items {
        if is_owned_by(&rs, &dep) {
            rs_api.delete(rs.name()).await?;
        }
    }
    Ok(Action::await_change())
}
```

### 古い ReplicaSet のクリーンアップ（reconcile_applied 内）

`replicas == 0` かつ `status.replicas == 0` の古い ReplicaSet を削除する（最大 `revision_history_limit` 件は保持してよい。デフォルト 0 件 = 即削除）：

```rust
for rs in old_replicasets {
    let has_no_ships = rs.status.as_ref().map(|s| s.replicas == 0).unwrap_or(true);
    let is_scaled_to_zero = rs.spec.replicas == Some(0);
    if is_scaled_to_zero && has_no_ships {
        rs_api.delete(rs.name()).await?;
    }
}
```

## 完了条件

- Deployment を削除すると配下の全 ReplicaSet が削除される
- ロールアウト完了後、古い ReplicaSet（replicas=0 かつ Ship なし）が削除される
- `cargo clippy` が通ること
