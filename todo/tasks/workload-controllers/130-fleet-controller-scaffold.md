# Fleet コントローラーの骨格作成

## 概要

`tugboat-controller-manager` に `FleetController` の骨格を追加する。
ReplicaSet/Deployment コントローラーと同じ構成で実装し、`main.rs` に登録する。

## 対象ファイル

- `tugboat-controller-manager/src/fleet.rs`（新規作成）
- `tugboat-controller-manager/src/main.rs`

## 実装内容

### `fleet.rs` を新規作成

```rust
use crate::base::TugboatController;
use crate::error::ControllerError;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::apps::v1::Fleet;

#[derive(Clone)]
struct FleetReconciler {
    client: TugboatClient,
}

pub(crate) struct FleetController {
    controller: Controller<Fleet>,
    reconciler: FleetReconciler,
}

impl FleetController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: FleetReconciler { client },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for FleetController {
    fn name(&self) -> &str { "fleet" }
    async fn setup(&mut self) {}
    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

#[async_trait::async_trait]
impl Reconciler<Fleet> for FleetReconciler {
    type Error = ControllerError;

    async fn reconcile(&self, event: ReconcileEvent<Fleet>) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(_fleet) => Ok(Action::await_change()),
            ReconcileEvent::Deleted(_fleet) => Ok(Action::await_change()),
        }
    }
}
```

### `main.rs` に登録

```rust
manager.add_controller(FleetController::new(client.clone()));
```

## 完了条件

- `cargo build --release --package tugboat-controller-manager` が通ること
- `cargo clippy` で警告がないこと
