// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Reconciler for `ShipSnapshot` resources.
//!
//! The state machine moves a `ShipSnapshot` through:
//!
//! ```text
//! Pending ──> Capturing ──> Ready
//!                    └────> Failed
//! ```
//!
//! Storage snapshot fanout (`spec.include_volumes`) is the
//! controller-manager's responsibility — the agent only consumes the
//! result via `status.volume_snapshots[].ready_to_use` and waits for
//! the set to be ready before issuing the VM snapshot.

#![allow(dead_code)]

use crate::reconciler::error::ReconcileError;
use crate::runtime::error::RuntimeError;
use async_trait::async_trait;
use tugboat_resources::manifests::core::v1::{
    Ship, ShipSnapshot, ShipSnapshotCondition, ShipSnapshotStatus, ShipSnapshotVolumeRef,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::snapshot::{VmSnapshotCreateResponse, VmSnapshotMode};

pub(crate) const PHASE_PENDING: &str = "Pending";
pub(crate) const PHASE_CAPTURING: &str = "Capturing";
pub(crate) const PHASE_READY: &str = "Ready";
pub(crate) const PHASE_FAILED: &str = "Failed";

pub(crate) const CONDITION_READY: &str = "Ready";
pub(crate) const CONDITION_FAILED: &str = "Failed";
pub(crate) const CONDITION_VOLUMES_PENDING: &str = "VolumeSnapshotsNotReady";

/// External dependencies the snapshot state machine needs to talk to.
/// Splitting these onto a trait keeps the state-machine code easy to
/// exercise with deterministic fakes.
#[async_trait]
pub(crate) trait SnapshotContext: Send + Sync {
    /// Returns the host node this reconciler is running on.
    fn node_name(&self) -> &str;

    /// Returns the Ship referenced by `spec.shipName`, or `None` when
    /// it does not (yet) exist.
    async fn get_ship(
        &self,
        namespace: &str,
        ship_name: &str,
    ) -> Result<Option<Ship>, ReconcileError>;

    /// Capture a VM-level snapshot via the active runtime.
    async fn snapshot_create(
        &self,
        ship_id: &str,
        mode: VmSnapshotMode,
    ) -> Result<VmSnapshotCreateResponse, RuntimeError>;

    /// Persist `status` on the `ShipSnapshot` object.
    async fn patch_status(
        &self,
        namespace: &str,
        name: &str,
        status: ShipSnapshotStatus,
    ) -> Result<(), ReconcileError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SnapshotAction {
    /// No action — the snapshot is not bound to this node yet, has
    /// already completed, or its dependencies are not ready. The
    /// agent should re-evaluate on the next watch event.
    Wait(WaitReason),
    /// The VM snapshot finished successfully.
    Ready,
    /// The snapshot terminated in a failed state.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WaitReason {
    /// Ship referenced by `spec.shipName` does not exist on this node.
    NotOnThisNode,
    /// Already terminal; no further work for the agent.
    AlreadyTerminal,
    /// `spec.include_volumes` is true but volume snapshots are not
    /// yet `readyToUse`.
    VolumeSnapshotsPending,
    /// Validation prevented forward progress (recorded on status).
    InvalidSpec,
}

pub(crate) struct SnapshotStateMachine<'a> {
    context: &'a dyn SnapshotContext,
}

impl<'a> SnapshotStateMachine<'a> {
    pub(crate) fn new(context: &'a dyn SnapshotContext) -> Self {
        Self { context }
    }

    pub(crate) async fn reconcile(
        &self,
        snapshot: &ShipSnapshot,
    ) -> Result<SnapshotAction, ReconcileError> {
        let namespace = snapshot
            .object_meta
            .as_ref()
            .and_then(|meta| meta.namespace.as_deref())
            .ok_or_else(|| {
                ReconcileError::FieldMissing(
                    "core/v1.ShipSnapshot".to_string(),
                    "metadata.namespace".to_string(),
                )
            })?;
        let name = snapshot
            .object_meta
            .as_ref()
            .and_then(|meta| meta.name.as_deref())
            .ok_or_else(|| {
                ReconcileError::FieldMissing(
                    "core/v1.ShipSnapshot".to_string(),
                    "metadata.name".to_string(),
                )
            })?;
        let Some(spec) = snapshot.spec.as_ref() else {
            return Err(ReconcileError::FieldMissing(
                "core/v1.ShipSnapshot".to_string(),
                "spec".to_string(),
            ));
        };

        if let Some(status) = snapshot.status.as_ref()
            && (status.phase == PHASE_READY || status.phase == PHASE_FAILED)
        {
            return Ok(SnapshotAction::Wait(WaitReason::AlreadyTerminal));
        }

        if spec.ship_name.trim().is_empty() {
            let message = "spec.shipName must not be empty".to_string();
            self.write_failed(namespace, name, &message).await?;
            return Ok(SnapshotAction::Wait(WaitReason::InvalidSpec));
        }

        let Some(ship) = self.context.get_ship(namespace, &spec.ship_name).await? else {
            return Ok(SnapshotAction::Wait(WaitReason::NotOnThisNode));
        };
        let ship_node = ship
            .spec
            .as_ref()
            .and_then(|spec| spec.node_name.as_deref());
        if ship_node != Some(self.context.node_name()) {
            return Ok(SnapshotAction::Wait(WaitReason::NotOnThisNode));
        }

        if spec.include_volumes.unwrap_or(false) {
            let status = snapshot.status.as_ref();
            let volume_snapshots = status
                .map(|status| status.volume_snapshots.as_slice())
                .unwrap_or(&[]);
            if !all_volume_snapshots_ready(volume_snapshots) {
                self.write_capturing_waiting_for_volumes(namespace, name, snapshot)
                    .await?;
                return Ok(SnapshotAction::Wait(WaitReason::VolumeSnapshotsPending));
            }
        }

        let mode = parse_mode(spec.mode.as_deref())?;
        let ship_id = ship
            .object_meta
            .as_ref()
            .and_then(|meta| meta.uid.as_deref())
            .ok_or_else(|| {
                ReconcileError::FieldMissing("core/v1.Ship".to_string(), "metadata.uid".to_string())
            })?
            .to_string();

        match self.context.snapshot_create(&ship_id, mode).await {
            Ok(response) => {
                self.write_ready(namespace, name, snapshot, response)
                    .await?;
                Ok(SnapshotAction::Ready)
            }
            Err(err) => {
                let message = err.to_string();
                self.write_failed(namespace, name, &message).await?;
                Ok(SnapshotAction::Failed(message))
            }
        }
    }

    async fn write_capturing_waiting_for_volumes(
        &self,
        namespace: &str,
        name: &str,
        snapshot: &ShipSnapshot,
    ) -> Result<(), ReconcileError> {
        let existing_volumes = snapshot
            .status
            .as_ref()
            .map(|status| status.volume_snapshots.clone())
            .unwrap_or_default();
        let conditions = vec![ShipSnapshotCondition {
            r#type: CONDITION_VOLUMES_PENDING.to_string(),
            status: "True".to_string(),
            message: "Waiting for VolumeSnapshots to become ready_to_use".to_string(),
            timestamp: Some(Time::now()),
        }];
        let status = ShipSnapshotStatus {
            phase: PHASE_CAPTURING.to_string(),
            creation_time: Some(Time::now()),
            handle: None,
            runtime: None,
            volume_snapshots: existing_volumes,
            error: None,
            size_bytes: None,
            conditions,
        };
        self.context.patch_status(namespace, name, status).await
    }

    async fn write_ready(
        &self,
        namespace: &str,
        name: &str,
        snapshot: &ShipSnapshot,
        response: VmSnapshotCreateResponse,
    ) -> Result<(), ReconcileError> {
        let existing_volumes = snapshot
            .status
            .as_ref()
            .map(|status| status.volume_snapshots.clone())
            .unwrap_or_default();
        let status = ShipSnapshotStatus {
            phase: PHASE_READY.to_string(),
            creation_time: Some(Time::now()),
            handle: Some(response.handle),
            runtime: Some(response.runtime),
            volume_snapshots: existing_volumes,
            error: None,
            size_bytes: response.size_bytes.and_then(|v| i64::try_from(v).ok()),
            conditions: vec![ShipSnapshotCondition {
                r#type: CONDITION_READY.to_string(),
                status: "True".to_string(),
                message: "VM snapshot captured".to_string(),
                timestamp: Some(Time::now()),
            }],
        };
        self.context.patch_status(namespace, name, status).await
    }

    async fn write_failed(
        &self,
        namespace: &str,
        name: &str,
        message: &str,
    ) -> Result<(), ReconcileError> {
        let status = ShipSnapshotStatus {
            phase: PHASE_FAILED.to_string(),
            creation_time: Some(Time::now()),
            handle: None,
            runtime: None,
            volume_snapshots: Vec::new(),
            error: Some(message.to_string()),
            size_bytes: None,
            conditions: vec![ShipSnapshotCondition {
                r#type: CONDITION_FAILED.to_string(),
                status: "True".to_string(),
                message: message.to_string(),
                timestamp: Some(Time::now()),
            }],
        };
        self.context.patch_status(namespace, name, status).await
    }
}

fn all_volume_snapshots_ready(refs: &[ShipSnapshotVolumeRef]) -> bool {
    if refs.is_empty() {
        return false;
    }
    refs.iter().all(|r| r.ready_to_use.unwrap_or(false))
}

fn parse_mode(value: Option<&str>) -> Result<VmSnapshotMode, ReconcileError> {
    match value {
        None | Some("Online") => Ok(VmSnapshotMode::Online),
        Some("Offline") => Ok(VmSnapshotMode::Offline),
        Some(other) => Err(ReconcileError::Validation(format!(
            "spec.mode must be 'Online' or 'Offline', got '{other}'"
        ))),
    }
}

// `Ship` is used only via the SnapshotContext trait. Keep the explicit
// `use` so the linker keeps the symbol in scope when the trait is used.
#[allow(dead_code)]
fn _ship_marker(_: &Ship) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tugboat_resources::manifests::core::v1::{Ship, ShipSnapshotSpec, ShipSpec};
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    struct FakeContext {
        node_name: String,
        ship: Option<Ship>,
        snapshot_outcome: Result<VmSnapshotCreateResponse, String>,
        statuses: Mutex<Vec<ShipSnapshotStatus>>,
    }

    impl FakeContext {
        fn new(node_name: &str) -> Self {
            Self {
                node_name: node_name.to_string(),
                ship: None,
                snapshot_outcome: Ok(VmSnapshotCreateResponse {
                    handle: "snap-1".to_string(),
                    runtime: "qemu".to_string(),
                    created_at: "2026-05-14T00:00:00Z".to_string(),
                    size_bytes: Some(1024),
                }),
                statuses: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SnapshotContext for FakeContext {
        fn node_name(&self) -> &str {
            &self.node_name
        }
        async fn get_ship(
            &self,
            _namespace: &str,
            _ship_name: &str,
        ) -> Result<Option<Ship>, ReconcileError> {
            Ok(self.ship.clone())
        }
        async fn snapshot_create(
            &self,
            _ship_id: &str,
            _mode: VmSnapshotMode,
        ) -> Result<VmSnapshotCreateResponse, RuntimeError> {
            self.snapshot_outcome
                .clone()
                .map_err(RuntimeError::Other)
        }
        async fn patch_status(
            &self,
            _namespace: &str,
            _name: &str,
            status: ShipSnapshotStatus,
        ) -> Result<(), ReconcileError> {
            self.statuses.lock().unwrap().push(status);
            Ok(())
        }
    }

    fn snapshot_object(ship_name: &str, include_volumes: bool) -> ShipSnapshot {
        ShipSnapshot {
            object_meta: Some(ObjectMeta {
                name: Some("snap-a".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSnapshotSpec {
                ship_name: ship_name.to_string(),
                include_volumes: Some(include_volumes),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn ship_on_node(node: &str, ship_name: &str, uid: &str) -> Ship {
        Ship {
            object_meta: Some(ObjectMeta {
                name: Some(ship_name.to_string()),
                namespace: Some("default".to_string()),
                uid: Some(uid.to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                image: String::new(),
                ship_class: "small".to_string(),
                node_name: Some(node.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn happy_path_reaches_ready() {
        let mut ctx = FakeContext::new("node-a");
        ctx.ship = Some(ship_on_node("node-a", "ship-a", "ship-uid"));
        let machine = SnapshotStateMachine::new(&ctx);
        let snapshot = snapshot_object("ship-a", false);
        let action = machine.reconcile(&snapshot).await.unwrap();
        assert_eq!(action, SnapshotAction::Ready);
        let statuses = ctx.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].phase, PHASE_READY);
        assert_eq!(statuses[0].handle.as_deref(), Some("snap-1"));
    }

    #[tokio::test]
    async fn runtime_failure_writes_failed_phase() {
        let mut ctx = FakeContext::new("node-a");
        ctx.ship = Some(ship_on_node("node-a", "ship-a", "ship-uid"));
        ctx.snapshot_outcome = Err("qemu unreachable".to_string());
        let machine = SnapshotStateMachine::new(&ctx);
        let snapshot = snapshot_object("ship-a", false);
        let action = machine.reconcile(&snapshot).await.unwrap();
        match action {
            SnapshotAction::Failed(message) => assert!(message.contains("qemu unreachable")),
            other => panic!("expected Failed, got {other:?}"),
        }
        let statuses = ctx.statuses.lock().unwrap();
        assert_eq!(statuses[0].phase, PHASE_FAILED);
        assert!(statuses[0].error.is_some());
    }

    #[tokio::test]
    async fn already_ready_skips_runtime_call() {
        let mut ctx = FakeContext::new("node-a");
        ctx.ship = Some(ship_on_node("node-a", "ship-a", "ship-uid"));
        let machine = SnapshotStateMachine::new(&ctx);
        let mut snapshot = snapshot_object("ship-a", false);
        snapshot.status = Some(ShipSnapshotStatus {
            phase: PHASE_READY.to_string(),
            ..Default::default()
        });
        let action = machine.reconcile(&snapshot).await.unwrap();
        assert_eq!(action, SnapshotAction::Wait(WaitReason::AlreadyTerminal));
        assert!(ctx.statuses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ship_on_different_node_is_skipped() {
        let mut ctx = FakeContext::new("node-a");
        ctx.ship = Some(ship_on_node("node-b", "ship-a", "ship-uid"));
        let machine = SnapshotStateMachine::new(&ctx);
        let snapshot = snapshot_object("ship-a", false);
        let action = machine.reconcile(&snapshot).await.unwrap();
        assert_eq!(action, SnapshotAction::Wait(WaitReason::NotOnThisNode));
        assert!(ctx.statuses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn include_volumes_waits_until_all_ready() {
        let mut ctx = FakeContext::new("node-a");
        ctx.ship = Some(ship_on_node("node-a", "ship-a", "ship-uid"));
        let machine = SnapshotStateMachine::new(&ctx);
        let mut snapshot = snapshot_object("ship-a", true);
        snapshot.status = Some(ShipSnapshotStatus {
            phase: PHASE_PENDING.to_string(),
            volume_snapshots: vec![ShipSnapshotVolumeRef {
                pvc_name: "data".to_string(),
                volume_snapshot_name: "snap-a-data".to_string(),
                ready_to_use: Some(false),
            }],
            ..Default::default()
        });
        let action = machine.reconcile(&snapshot).await.unwrap();
        assert_eq!(
            action,
            SnapshotAction::Wait(WaitReason::VolumeSnapshotsPending)
        );
        let statuses = ctx.statuses.lock().unwrap();
        assert_eq!(statuses.last().unwrap().phase, PHASE_CAPTURING);
    }

    #[tokio::test]
    async fn include_volumes_proceeds_when_all_ready() {
        let mut ctx = FakeContext::new("node-a");
        ctx.ship = Some(ship_on_node("node-a", "ship-a", "ship-uid"));
        let machine = SnapshotStateMachine::new(&ctx);
        let mut snapshot = snapshot_object("ship-a", true);
        snapshot.status = Some(ShipSnapshotStatus {
            phase: PHASE_CAPTURING.to_string(),
            volume_snapshots: vec![ShipSnapshotVolumeRef {
                pvc_name: "data".to_string(),
                volume_snapshot_name: "snap-a-data".to_string(),
                ready_to_use: Some(true),
            }],
            ..Default::default()
        });
        let action = machine.reconcile(&snapshot).await.unwrap();
        assert_eq!(action, SnapshotAction::Ready);
    }

    #[tokio::test]
    async fn empty_ship_name_records_failure() {
        let ctx = FakeContext::new("node-a");
        let machine = SnapshotStateMachine::new(&ctx);
        let snapshot = snapshot_object("", false);
        let action = machine.reconcile(&snapshot).await.unwrap();
        assert_eq!(action, SnapshotAction::Wait(WaitReason::InvalidSpec));
        let statuses = ctx.statuses.lock().unwrap();
        assert_eq!(statuses[0].phase, PHASE_FAILED);
    }

    #[test]
    fn parse_mode_accepts_known_values() {
        assert_eq!(parse_mode(None).unwrap(), VmSnapshotMode::Online);
        assert_eq!(parse_mode(Some("Online")).unwrap(), VmSnapshotMode::Online);
        assert_eq!(
            parse_mode(Some("Offline")).unwrap(),
            VmSnapshotMode::Offline
        );
        assert!(parse_mode(Some("Fast")).is_err());
    }
}
