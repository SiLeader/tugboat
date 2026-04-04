# Deployment コントローラーの骨格作成

## 概要

`tugboat-controller-manager` に `DeploymentController` の骨格を追加する。
ReplicaSet コントローラーと同じ構成で `TugboatController` を実装し、`main.rs` に登録する。
この段階では reconcile ロジックは空でよい。

## 対象ファイル

- `tugboat-controller-manager/src/deployment.rs`（新規作成）
- `tugboat-controller-manager/src/main.rs`

## 実装内容

### `deployment.rs` を新規作成

```rust
use crate::base::TugboatController;
use crate::error::ControllerError;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::apps::v1::Deployment;

#[derive(Clone)]
struct DeploymentReconciler {
    client: TugboatClient,
}

pub(crate) struct DeploymentController {
    controller: Controller<Deployment>,
    reconciler: DeploymentReconciler,
}

impl DeploymentController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: DeploymentReconciler { client },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for DeploymentController {
    fn name(&self) -> &str { "deployment" }
    async fn setup(&mut self) {}
    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

#[async_trait::async_trait]
impl Reconciler<Deployment> for DeploymentReconciler {
    type Error = ControllerError;

    async fn reconcile(&self, event: ReconcileEvent<Deployment>) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(_dep) => Ok(Action::await_change()),
            ReconcileEvent::Deleted(_dep) => Ok(Action::await_change()),
        }
    }
}
```

### `main.rs` に登録

```rust
manager.add_controller(DeploymentController::new(client.clone()));
```

## 完了条件

- `cargo build --release --package tugboat-controller-manager` が通ること
- `cargo clippy` で警告がないこと
