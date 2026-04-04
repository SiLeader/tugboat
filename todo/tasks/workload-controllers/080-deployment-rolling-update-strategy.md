# Deployment: RollingUpdate ロールアウト戦略の実装

## 概要

`DeploymentStrategy.type == "RollingUpdate"`（またはデフォルト）のときのロールアウトを実装する。
インプレース更新パスとローテーションパスで挙動が異なる。

## 対象ファイル

- `tugboat-controller-manager/src/deployment.rs`

## RollingUpdate の挙動

### インプレースパスの場合（ship_class 変更など）

ReplicaSet コントローラー（025 タスク）が Ship を **1 台ずつ** spec 更新する。
エージェントがホットプラグやマイグレーションで対応すれば、Ship はダウンしない。
Deployment コントローラー側では以下を確認する：

1. 既存 ReplicaSet のテンプレートが更新済みであることを確認（060 タスクで実施済み）
2. ReplicaSet の `status.ready_replicas` を監視し、全 Ship が Ready に戻るまで待つ
3. 途中で Ship が Not Ready になった場合は `maxUnavailable` を超えないか確認する

```rust
async fn reconcile_rolling_update_in_place(
    &self,
    dep: &Deployment,
    active_rs: &ReplicaSet,
) -> Result<Action, ControllerError> {
    let desired = dep.spec.replicas.unwrap_or(1);
    let ready = active_rs.status.as_ref().map(|s| s.ready_replicas).unwrap_or(0);

    if ready < desired {
        // ReplicaSet コントローラーが Ship を更新中。完了を待つ
        return Ok(Action::requeue(Duration::from_secs(5)));
    }
    // 全 Ship が Ready → 更新完了
    Ok(Action::await_change())
}
```

### ローテーションパスの場合（image 変更など）

新旧 ReplicaSet を `maxSurge` / `maxUnavailable` に従って段階的にスケールする。

- `maxSurge`（デフォルト 1）: desired を超えて同時に存在できる Ship の最大数
- `maxUnavailable`（デフォルト 1）: desired に対して同時に不足できる Ship の最大数

```rust
async fn reconcile_rolling_update_rotation(
    &self,
    dep: &Deployment,
    active_rs: &ReplicaSet,
    old_replicasets: &[ReplicaSet],
) -> Result<Action, ControllerError> {
    let desired = dep.spec.replicas.unwrap_or(1);
    let strategy = dep.spec.strategy.as_ref();
    let max_surge = strategy.and_then(|s| s.rolling_update.as_ref())
        .and_then(|r| r.max_surge).unwrap_or(1);
    let max_unavailable = strategy.and_then(|s| s.rolling_update.as_ref())
        .and_then(|r| r.max_unavailable).unwrap_or(1);

    let old_replicas: i32 = old_replicasets.iter()
        .filter_map(|rs| rs.status.as_ref().map(|s| s.replicas))
        .sum();
    let new_replicas = active_rs.status.as_ref().map(|s| s.replicas).unwrap_or(0);
    let total = old_replicas + new_replicas;

    // スケールアップ: 新 RS の replicas を増やす
    let can_add = (desired + max_surge) - total;
    if can_add > 0 {
        let new_desired = (active_rs.spec.replicas.unwrap_or(0) + can_add).min(desired);
        // active_rs.spec.replicas = new_desired に更新
    }

    // スケールダウン: 新 RS の ready が十分なら古い RS を減らす
    let new_ready = active_rs.status.as_ref().map(|s| s.ready_replicas).unwrap_or(0);
    let min_available = desired - max_unavailable;
    if new_ready >= min_available.max(0) && old_replicas > 0 {
        // 古い RS の replicas を 1 ずつ減らす
    }

    if old_replicas > 0 || new_replicas < desired {
        return Ok(Action::requeue(Duration::from_secs(2)));
    }
    Ok(Action::await_change())
}
```

### パスの分岐

```rust
async fn reconcile_rolling_update(
    &self,
    dep: &Deployment,
    active_rs: Option<&ReplicaSet>,
    old_replicasets: &[ReplicaSet],
    change_kind: TemplateChangeKind,
) -> Result<Action, ControllerError> {
    let Some(active_rs) = active_rs else {
        return Ok(Action::await_change()); // RS 未作成（060 で作成される）
    };

    match change_kind {
        TemplateChangeKind::NoChange | TemplateChangeKind::InPlace => {
            self.reconcile_rolling_update_in_place(dep, active_rs).await
        }
        TemplateChangeKind::RequiresRotation => {
            self.reconcile_rolling_update_rotation(dep, active_rs, old_replicasets).await
        }
    }
}
```

## 完了条件

- インプレースパス: Ship が 1 台ずつ更新され、全 Ship が Ready になるまで Deployment が Progressing を報告する
- ローテーションパス: 新旧 RS が段階的に入れ替わる
- `maxUnavailable = 0` のとき、既存 Ship が常に desired 数を維持しながらロールアウトされる
- `cargo clippy` が通ること
