# ReplicaSet: 余剰 Ship の削除

## 概要

desired replicas より多い Ship が存在する場合に、余剰分の Ship を削除するロジックを実装する。
また、ReplicaSet が削除された場合（`ReconcileEvent::Deleted`）にも所有 Ship をすべて削除する。

## 対象ファイル

- `tugboat-controller-manager/src/replicaset.rs`

## 実装内容

### 余剰削除（reconcile_applied 内）

```rust
if owned_ships.len() > desired {
    let excess = owned_ships.len() - desired;
    // 新しいものから削除（作成時刻の降順でソートして末尾から取る）
    let to_delete = &owned_ships[desired..];
    for ship in to_delete.iter().take(excess) {
        ship_api.delete(ship.name()).await?;
    }
}
```

削除順は原則として creation_timestamp の降順（最近作成されたものから削除）とする。

### ReplicaSet 削除時の Ship 全削除（reconcile_deleted）

```rust
async fn reconcile_deleted(&self, rs: ReplicaSet) -> Result<Action, ControllerError> {
    let namespace = rs.namespace().unwrap_or_default();
    let ship_api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
    let ships = ship_api.list().await?;
    for ship in ships.items {
        if is_owned_by(&ship, &rs) {
            ship_api.delete(ship.name()).await?;
        }
    }
    Ok(Action::await_change())
}
```

`is_owned_by` は `owner_references` に ReplicaSet の uid が含まれているかで判定する。

## 完了条件

- desired より多い Ship があるとき、余剰分が削除される
- ReplicaSet を削除したとき、所有する Ship がすべて削除される
- 404 Not Found は冪等として無視する
- `cargo clippy` および `cargo test` が通ること
