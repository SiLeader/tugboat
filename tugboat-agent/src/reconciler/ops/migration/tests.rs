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

use super::super::{PHASE_MIGRATING, PHASE_PENDING, PHASE_READY};
use super::*;
use crate::csi::READ_WRITE_MANY;
use crate::reconciler::volume::VolumeInfo;
use preflight::normalize_architecture;
use std::sync::Mutex;
use tugboat_resources::manifests::core::v1::{
    CpuSpec, CsiPersistentVolumeSource, MemorySpec, NodeOvercommitSpec, NodeResource, NodeSpec,
    PersistentVolumeClaimSpec, PersistentVolumeSpec, ShipClassSpec, ShipSpec, ShipStatus,
};
use tugboat_resources::manifests::meta::v1::ObjectMeta;
use tugboat_vm_runtime_interface::migrate::VmMigrationPhase;

#[derive(Debug, Clone)]
struct StatusUpdate {
    migration: ShipMigrationStatus,
    condition_status: String,
    condition_message: String,
}

struct FakeContext {
    node_name: String,
    has_ship: bool,
    migration_phase: VmMigrationPhase,
    runtime_status: Option<VmStatus>,
    preflight: MigrationPreflight,
    updates: Mutex<Vec<StatusUpdate>>,
    migrate_calls: Mutex<Vec<(String, u16)>>,
    finish_calls: Mutex<usize>,
    cancel_calls: Mutex<usize>,
    ship_patches: Mutex<Vec<serde_json::Value>>,
}

#[async_trait]
impl MigrationContext for FakeContext {
    fn node_name(&self) -> &str {
        &self.node_name
    }
    async fn has_ship(&self, _ship_id: &str) -> bool {
        self.has_ship
    }
    async fn reconcile_deleted(&self, _ship: Ship) -> Result<(), ReconcileError> {
        Ok(())
    }
    async fn preflight_migration(
        &self,
        _ship: &Ship,
        _target_node_name: &str,
    ) -> Result<MigrationPreflight, ReconcileError> {
        Ok(self.preflight.clone())
    }
    async fn migrate(
        &self,
        _ship_id: &str,
        addr: String,
        port: u16,
        _params: VmMigrationParams,
    ) -> Result<(), RuntimeError> {
        self.migrate_calls.lock().unwrap().push((addr, port));
        Ok(())
    }
    async fn resolve_migration_params(&self, _ship: &Ship) -> VmMigrationParams {
        VmMigrationParams::default()
    }
    async fn check_migration_status(
        &self,
        _ship_id: &str,
    ) -> Result<VmMigrationStatusResponse, RuntimeError> {
        Ok(VmMigrationStatusResponse {
            phase: self.migration_phase.clone(),
            message: format!("{:?}", &self.migration_phase),
            bytes_transferred: None,
            bytes_remaining: None,
            ram_dirty_rate_mbps: None,
        })
    }
    async fn finish_source_migration(&self, _ship_id: &str) -> Result<(), RuntimeError> {
        *self.finish_calls.lock().unwrap() += 1;
        Ok(())
    }
    async fn cancel_migration(&self, _ship_id: &str) -> Result<(), RuntimeError> {
        *self.cancel_calls.lock().unwrap() += 1;
        Ok(())
    }
    async fn local_runtime_status(&self, _ship_id: &str) -> Result<Option<VmStatus>, RuntimeError> {
        Ok(self.runtime_status)
    }

    async fn update_migration_status(
        &self,
        _namespace: &str,
        _name: &str,
        migration: ShipMigrationStatus,
        condition_status: &str,
        condition_message: String,
    ) -> Result<(), ReconcileError> {
        self.updates.lock().unwrap().push(StatusUpdate {
            migration,
            condition_status: condition_status.to_string(),
            condition_message,
        });
        Ok(())
    }

    async fn patch_ship(
        &self,
        _namespace: &str,
        _name: &str,
        patch: serde_json::Value,
    ) -> Result<(), ReconcileError> {
        self.ship_patches.lock().unwrap().push(patch);
        Ok(())
    }
}

fn fake_context(node_name: &str, migration_phase: VmMigrationPhase) -> FakeContext {
    FakeContext {
        node_name: node_name.to_string(),
        has_ship: true,
        migration_phase,
        runtime_status: Some(VmStatus::Paused),
        preflight: MigrationPreflight::Ready,
        updates: Mutex::new(Vec::new()),
        migrate_calls: Mutex::new(Vec::new()),
        finish_calls: Mutex::new(0),
        cancel_calls: Mutex::new(0),
        ship_patches: Mutex::new(Vec::new()),
    }
}

fn stale_timestamp(age_secs: i64) -> Time {
    let now = Time::now();
    Time {
        seconds: now.seconds - age_secs,
        nanos: now.nanos,
    }
}

#[tokio::test]
async fn test_migration_not_for_me() {
    let context = fake_context("node-1", VmMigrationPhase::Completed);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-2".to_string()),
            target_node_name: Some("node-3".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(!result);
}

#[tokio::test]
async fn test_migration_source_initial() {
    let context = fake_context("node-1", VmMigrationPhase::Completed);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    // Should initiate migration (update status to Pending) and return true
    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);
}

#[tokio::test]
async fn test_migration_source_ready() {
    let context = fake_context("node-1", VmMigrationPhase::None);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_READY.to_string(),
                target_address: Some("1.2.3.4".to_string()),
                target_port: Some(1234),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);
    assert_eq!(context.migrate_calls.lock().unwrap().len(), 1);
    assert_eq!(
        context
            .updates
            .lock()
            .unwrap()
            .last()
            .map(|update| update.migration.phase.as_str()),
        Some(PHASE_MIGRATING)
    );
    assert!(
        context
            .updates
            .lock()
            .unwrap()
            .last()
            .and_then(|update| update.migration.timestamp.as_ref())
            .is_some()
    );
}

#[tokio::test]
async fn test_migration_source_migrating_completed() {
    let context = fake_context("node-1", VmMigrationPhase::Completed);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_MIGRATING.to_string(),
                target_address: Some("1.2.3.4".to_string()),
                target_port: Some(1234),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);
    assert_eq!(*context.finish_calls.lock().unwrap(), 1);
    assert_eq!(context.ship_patches.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_migration_target_failed_cleanup() {
    let context = fake_context("target-node", VmMigrationPhase::Failed);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("source-node".to_string()),
            target_node_name: Some("target-node".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_FAILED.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    // Should handle cleanup on target node and return true
    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);
}

#[tokio::test]
async fn test_migration_preflight_rejection_marks_ship_failed() {
    let context = FakeContext {
        preflight: MigrationPreflight::Reject("target node is not CNI-ready".to_string()),
        ..fake_context("node-1", VmMigrationPhase::Completed)
    };
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);

    let updates = context.updates.lock().unwrap();
    let update = updates.last().expect("expected migration status update");
    assert_eq!(update.migration.phase, PHASE_FAILED);
    assert_eq!(update.condition_status, "VmMigrationPreflightFailed");
    assert!(
        update
            .condition_message
            .contains("Source VM remains on the source node")
    );
}

#[tokio::test]
async fn test_migration_source_ready_detects_existing_active_migration() {
    let context = fake_context("node-1", VmMigrationPhase::Active);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_READY.to_string(),
                target_address: Some("1.2.3.4".to_string()),
                target_port: Some(1234),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);
    assert!(context.migrate_calls.lock().unwrap().is_empty());
    assert_eq!(
        context
            .updates
            .lock()
            .unwrap()
            .last()
            .map(|update| update.migration.phase.as_str()),
        Some(PHASE_MIGRATING)
    );
}

#[tokio::test]
async fn test_migration_completed_without_runtime_still_finalizes_cutover() {
    let context = FakeContext {
        has_ship: false,
        ..fake_context("node-1", VmMigrationPhase::Completed)
    };
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_COMPLETED.to_string(),
                target_address: Some("1.2.3.4".to_string()),
                target_port: Some(1234),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);
    assert_eq!(*context.finish_calls.lock().unwrap(), 0);
    assert_eq!(context.ship_patches.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_running_target_runtime_finalizes_cutover_after_source_loss() {
    let context = FakeContext {
        node_name: "target-node".to_string(),
        runtime_status: Some(VmStatus::Running),
        ..fake_context("target-node", VmMigrationPhase::None)
    };
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("source-node".to_string()),
            target_node_name: Some("target-node".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_MIGRATING.to_string(),
                source_node_name: Some("source-node".to_string()),
                target_node_name: Some("target-node".to_string()),
                target_address: Some("1.2.3.4".to_string()),
                target_port: Some(1234),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);
    assert_eq!(*context.finish_calls.lock().unwrap(), 0);
    assert_eq!(context.ship_patches.lock().unwrap().len(), 1);
    assert_eq!(
        context
            .updates
            .lock()
            .unwrap()
            .last()
            .map(|update| update.migration.phase.as_str()),
        Some(PHASE_COMPLETED)
    );
}

#[tokio::test]
async fn test_pending_timeout_marks_failed_and_cancels() {
    let context = fake_context("node-1", VmMigrationPhase::None);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_PENDING.to_string(),
                timestamp: Some(stale_timestamp(MIGRATION_PENDING_TIMEOUT_SECS + 10)),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);
    assert_eq!(*context.cancel_calls.lock().unwrap(), 1);
    let updates = context.updates.lock().unwrap();
    assert_eq!(
        updates.last().map(|u| u.migration.phase.as_str()),
        Some(PHASE_FAILED)
    );
    assert!(
        updates
            .last()
            .unwrap()
            .condition_message
            .contains("did not become ready")
    );
}

#[tokio::test]
async fn test_no_timeout_when_pending_timestamp_is_recent() {
    let context = fake_context("node-1", VmMigrationPhase::None);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_PENDING.to_string(),
                timestamp: Some(Time::now()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);
    assert_eq!(*context.cancel_calls.lock().unwrap(), 0);
    assert!(context.updates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_migrating_timeout_marks_failed_and_cancels() {
    let context = fake_context("node-1", VmMigrationPhase::Active);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_MIGRATING.to_string(),
                target_address: Some("1.2.3.4".to_string()),
                target_port: Some(1234),
                timestamp: Some(stale_timestamp(MIGRATION_ACTIVE_TIMEOUT_SECS + 10)),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = sm.try_reconcile(&ship, "ship-1").await;
    // Timeout returns an Err (MigrationFailed)
    assert!(result.is_err());
    assert_eq!(*context.cancel_calls.lock().unwrap(), 1);
    let updates = context.updates.lock().unwrap();
    assert_eq!(
        updates.last().map(|u| u.migration.phase.as_str()),
        Some(PHASE_FAILED)
    );
    assert!(
        updates
            .last()
            .unwrap()
            .condition_message
            .contains("exceeded the maximum allowed time")
    );
}

#[tokio::test]
async fn test_migrating_status_update_preserves_existing_timestamp() {
    let context = fake_context("node-1", VmMigrationPhase::Active);
    let sm = MigrationStateMachine::new(&context);
    let timestamp = stale_timestamp(120);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_MIGRATING.to_string(),
                target_address: Some("1.2.3.4".to_string()),
                target_port: Some(1234),
                timestamp: Some(timestamp),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = sm.try_reconcile(&ship, "ship-1").await.unwrap();
    assert!(result);

    let updates = context.updates.lock().unwrap();
    let update = updates.last().expect("expected migration status update");
    assert_eq!(update.migration.phase, PHASE_MIGRATING);
    assert_eq!(update.migration.timestamp, Some(timestamp));
}

#[tokio::test]
async fn test_cancel_called_when_qemu_reports_failed_during_migrating() {
    let context = fake_context("node-1", VmMigrationPhase::Failed);
    let sm = MigrationStateMachine::new(&context);
    let ship = Ship {
        object_meta: Some(ObjectMeta {
            name: Some("ship-1".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipSpec {
            node_name: Some("node-1".to_string()),
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_MIGRATING.to_string(),
                target_address: Some("1.2.3.4".to_string()),
                target_port: Some(1234),
                timestamp: Some(Time::now()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let result = sm.try_reconcile(&ship, "ship-1").await;
    assert!(result.is_err());
    assert_eq!(*context.cancel_calls.lock().unwrap(), 1);
}

#[test]
fn normalizes_common_architecture_aliases() {
    assert_eq!(normalize_architecture("x86_64"), "amd64");
    assert_eq!(normalize_architecture("amd64"), "amd64");
    assert_eq!(normalize_architecture("aarch64"), "arm64");
    assert_eq!(normalize_architecture("arm64"), "arm64");
}

#[test]
fn rejects_non_shared_persistent_volumes_for_live_migration() {
    let volume = VolumeInfo::PersistentVolumeClaim(Box::new(
        crate::reconciler::volume::PersistentVolumeClaimVolumeInfo {
            name: "data".to_string(),
            claim_name: "data-pvc".to_string(),
            volume_name: "data-pv".to_string(),
            claim: PersistentVolumeClaimSpec {
                access_modes: vec!["ReadWriteOnce".to_string()],
                ..Default::default()
            },
            volume: PersistentVolumeSpec {
                access_modes: vec!["ReadWriteOnce".to_string()],
                csi: Some(CsiPersistentVolumeSource::default()),
                ..Default::default()
            },
            status: None,
            source: CsiPersistentVolumeSource::default(),
        },
    ));

    let reason =
        validate_storage_eligibility(&[volume]).expect("non-shared storage should be rejected");
    assert!(reason.contains(READ_WRITE_MANY));
}

#[test]
fn rejects_target_node_without_enough_cpu_capacity_for_live_migration() {
    let target_node = Node {
        object_meta: Some(ObjectMeta {
            name: Some("node-2".to_string()),
            ..Default::default()
        }),
        spec: Some(NodeSpec {
            overcommit: Some(NodeOvercommitSpec {
                cpu_ratio: "1".to_string(),
                memory_ratio: "1".to_string(),
            }),
            resource: Some(NodeResource {
                cpu: 4,
                memory: 8 * 1024 * 1024 * 1024,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let incumbent_class = ShipClass {
        object_meta: Some(ObjectMeta {
            name: Some("incumbent".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipClassSpec {
            cpu: Some(CpuSpec {
                architecture: "amd64".to_string(),
                cores: 3,
            }),
            memory: Some(MemorySpec {
                size: "2Gi".to_string(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let migrating_class = ShipClass {
        object_meta: Some(ObjectMeta {
            name: Some("migrating".to_string()),
            ..Default::default()
        }),
        spec: Some(ShipClassSpec {
            cpu: Some(CpuSpec {
                architecture: "amd64".to_string(),
                cores: 2,
            }),
            memory: Some(MemorySpec {
                size: "1Gi".to_string(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let resident_ship = Ship {
        spec: Some(ShipSpec {
            node_name: Some("node-2".to_string()),
            ship_class: "incumbent".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let reason = validate_target_resource_capacity(
        &target_node,
        &[resident_ship],
        &[incumbent_class, migrating_class.clone()],
        &migrating_class,
    )
    .expect("insufficient CPU capacity should be rejected");

    assert!(reason.contains("insufficient CPU"));
    assert!(reason.contains("requested=2"));
    assert!(reason.contains("available=1"));
}

#[test]
fn upsert_ship_condition_preserves_unrelated_conditions() {
    let timestamp = Time::now();
    let mut conditions = vec![
        ShipCondition {
            status: "Running".to_string(),
            message: "VM is running".to_string(),
            timestamp: Some(timestamp),
        },
        ShipCondition {
            status: "VmMigrationPending".to_string(),
            message: "old migration state".to_string(),
            timestamp: Some(timestamp),
        },
    ];

    upsert_ship_condition(
        &mut conditions,
        ShipCondition {
            status: "VmMigrationPending".to_string(),
            message: "new migration state".to_string(),
            timestamp: Some(Time::now()),
        },
    );

    assert_eq!(conditions.len(), 2);
    assert!(conditions.iter().any(|condition| {
        condition.status == "Running" && condition.message == "VM is running"
    }));
    assert!(conditions.iter().any(|condition| {
        condition.status == "VmMigrationPending" && condition.message == "new migration state"
    }));
}
