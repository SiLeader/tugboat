# Deployment ステータスの更新

## 概要

reconcile の最後に `DeploymentStatus` を計算して API サーバーに書き戻す。

## 対象ファイル

- `tugboat-controller-manager/src/deployment.rs`

## ステータスフィールドの定義（deployment.proto より）

```protobuf
message DeploymentStatus {
  int32 replicas = 1;          // 全 Ship（新旧合計）
  int32 ready_replicas = 2;    // Ready な Ship 数
  int32 updated_replicas = 3;  // 新 ReplicaSet が管理する Ship 数
}
```

## 実装内容

### ステータス計算

- `replicas`: 所有するすべての ReplicaSet の `status.replicas` の合計
- `ready_replicas`: 所有するすべての ReplicaSet の `status.ready_replicas` の合計
- `updated_replicas`: アクティブ ReplicaSet の `status.replicas`

### 書き戻し

```rust
let new_status = DeploymentStatus {
    replicas: total_replicas,
    ready_replicas: total_ready,
    updated_replicas: active_replicas,
};

if dep.status.as_ref() != Some(&new_status) {
    let mut updated = dep.clone();
    updated.status = Some(new_status);
    let dep_api: Api<Deployment> = Api::namespaced(self.client.clone(), namespace);
    dep_api.replace(dep.name(), updated).await?;
}
```

## 完了条件

- `status.replicas` が全所有 Ship 数と一致する
- `status.ready_replicas` が Ready な Ship 数と一致する
- `status.updated_replicas` がアクティブ ReplicaSet の Ship 数と一致する
- ステータスが変化していないときは API 呼び出しをしない
- `cargo clippy` および `cargo test` が通ること
