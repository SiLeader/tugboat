# Deployment: Recreate ロールアウト戦略の実装

## 概要

`DeploymentStrategy.type == "Recreate"` のときのロールアウトを実装する。
Tugboat ではインプレース更新パスとローテーションパスの 2 つがあるため、
それぞれの Recreate の挙動が異なる。

## 対象ファイル

- `tugboat-controller-manager/src/deployment.rs`

## Recreate の挙動

### ローテーションパスの場合（image 変更など）

1. 古い ReplicaSet の `replicas` を 0 に設定し、古い Ship がすべて停止するまで待つ
2. 古い Ship がゼロになったら、新しい ReplicaSet の `replicas` を desired 値に設定する

### インプレースパスの場合（ship_class 変更など）

1. 既存 ReplicaSet のテンプレートが既に更新済み（060 タスクで実施）
2. ReplicaSet コントローラー（025 タスク）が Ship を 1 台ずつ更新するが、
   Recreate 戦略では **全 Ship を一斉に更新**させたい
3. そのため ReplicaSet に `update-strategy: all` のようなアノテーションを設定し、
   ReplicaSet コントローラーが一斉更新モードで動作するようにする

> **代替案**: Recreate + インプレースの場合でも ReplicaSet コントローラーの
> ローリング更新に任せる（ダウンタイムを最小化するため）。
> エージェントがホットプラグで対応できるなら一斉更新でもダウンタイムは発生しない。

## 実装内容

### reconcile_applied 内での分岐

```rust
let strategy_type = dep.spec.strategy.as_ref()
    .map(|s| s.r#type.as_str())
    .unwrap_or("RollingUpdate");

match strategy_type {
    "Recreate" => self.reconcile_recreate(&dep, active_rs.as_ref(), &old_replicasets, change_kind).await?,
    _ => self.reconcile_rolling_update(&dep, active_rs.as_ref(), &old_replicasets, change_kind).await?,
}
```

### reconcile_recreate（ローテーション）

```rust
async fn reconcile_recreate_rotation(
    &self,
    dep: &Deployment,
    active_rs: &ReplicaSet,
    old_replicasets: &[ReplicaSet],
) -> Result<Action, ControllerError> {
    // 1. 古い ReplicaSet を 0 にスケールダウン
    let old_running = old_replicasets.iter()
        .any(|rs| rs.status.as_ref().map(|s| s.replicas > 0).unwrap_or(false));
    if old_running {
        for rs in old_replicasets {
            if rs.spec.replicas != Some(0) {
                let mut updated = rs.clone();
                updated.spec.replicas = Some(0);
                rs_api.replace(rs.name(), updated).await?;
            }
        }
        return Ok(Action::requeue(Duration::from_secs(2)));
    }

    // 2. 新 RS をスケールアップ
    let desired = dep.spec.replicas.unwrap_or(1);
    if active_rs.spec.replicas != Some(desired) {
        let mut updated = active_rs.clone();
        updated.spec.replicas = Some(desired);
        rs_api.replace(active_rs.name(), updated).await?;
    }
    Ok(Action::await_change())
}
```

## 完了条件

- ローテーションパス: 古い Ship が全て停止してから新しい Ship が起動する
- インプレースパス: 全 Ship の spec が更新される（エージェントがホットプラグで対応すればダウンタイムなし）
- `cargo clippy` が通ること
