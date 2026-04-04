# ReplicaSet コントローラーの骨格作成

## 概要

`tugboat-controller-manager` に `ReplicaSetController` の骨格を追加する。
既存の `PvcProvisionerController` と同じパターンで `TugboatController` を実装し、
`main.rs` に登録する。この段階では reconcile ロジックは空（`Action::await_change()` を返すのみ）でよい。

## 対象ファイル

- `tugboat-controller-manager/src/replicaset.rs`（新規作成）
- `tugboat-controller-manager/src/main.rs`
- `tugboat-controller-manager/Cargo.toml`（必要なら依存追加）

## 実装内容

### 1. `replicaset.rs` を新規作成

```rust
use crate::base::TugboatController;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::apps::v1::ReplicaSet;

#[derive(Clone)]
struct ReplicaSetReconciler {
    client: TugboatClient,
}

pub(crate) struct ReplicaSetController {
    controller: Controller<ReplicaSet>,
    reconciler: ReplicaSetReconciler,
}

impl ReplicaSetController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: ReplicaSetReconciler { client },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for ReplicaSetController {
    fn name(&self) -> &str { "replicaset" }
    async fn setup(&mut self) {}
    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

#[async_trait::async_trait]
impl Reconciler<ReplicaSet> for ReplicaSetReconciler {
    type Error = crate::error::ControllerError;

    async fn reconcile(&self, event: ReconcileEvent<ReplicaSet>) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(_rs) => Ok(Action::await_change()),
            ReconcileEvent::Deleted(_rs) => Ok(Action::await_change()),
        }
    }
}
```

### 2. `main.rs` でコントローラーを登録

`PvcProvisionerController` が登録されているのと同じ場所に追加：

```rust
manager.add_controller(ReplicaSetController::new(client.clone()));
```

## 完了条件

- `cargo build --release --package tugboat-controller-manager` が通ること
- `cargo clippy` で警告がないこと
