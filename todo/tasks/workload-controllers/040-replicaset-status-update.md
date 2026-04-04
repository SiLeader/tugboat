# ReplicaSet ステータスの更新

## 概要

reconcile の最後に `ReplicaSetStatus` を計算して API サーバーに書き戻す。

## 対象ファイル

- `tugboat-controller-manager/src/replicaset.rs`

## ステータスフィールドの定義（replica_set.proto より）

```protobuf
message ReplicaSetStatus {
  int32 replicas = 1;         // 所有 Ship の総数
  int32 ready_replicas = 2;   // Ready 状態の Ship 数
}
```

## 実装内容

### Ready 判定

Ship が「Ready」とみなせる条件を定義する。
`ShipStatus.conditions` に `type = "Ready"` かつ `status = "True"` の Condition が存在すること。

### ステータス書き戻し

```rust
let new_status = ReplicaSetStatus {
    replicas: owned_ships.len() as i32,
    ready_replicas: ready_count as i32,
};

// 変化があるときだけ書き戻す
if rs.status.as_ref() != Some(&new_status) {
    let mut updated = rs.clone();
    updated.status = Some(new_status);
    let rs_api: Api<ReplicaSet> = Api::namespaced(self.client.clone(), namespace);
    rs_api.replace(rs.name(), updated).await?;
}
```

## 完了条件

- `status.replicas` が所有 Ship 数と一致する
- `status.ready_replicas` が Ready 条件を持つ Ship 数と一致する
- ステータスが変化していないときは API 呼び出しをしない（不要な書き込みを避ける）
- `cargo clippy` および `cargo test` が通ること
