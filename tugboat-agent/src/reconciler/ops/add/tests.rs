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

use super::recovery::{
    CleanupSecretPolicy, RecoveredRuntimeAction, best_effort_stale_volume_cleanup,
    controller_publish_secrets_for_cleanup, find_recovered_published_volume,
    recovered_runtime_action,
};
use crate::csi::{PublishedAccessType, PublishedVolume};
use crate::reconciler::error::ReconcileError;
use crate::reconciler::ops::add_helpers::validate_recovered_published_volumes;
use crate::reconciler::ops::{PHASE_COMPLETED, PHASE_FAILED, PHASE_PENDING, PHASE_READY};
use std::collections::HashMap;
use tugboat_resources::manifests::core::v1::{Ship, ShipMigrationStatus, ShipSpec, ShipStatus};

#[test]
fn best_effort_stale_volume_cleanup_keeps_success_values() {
    let result = best_effort_stale_volume_cleanup("default", "claim-1", Ok(42_u8));

    assert_eq!(result, Some(42));
}

#[test]
fn best_effort_stale_volume_cleanup_drops_errors() {
    let result: Option<()> = best_effort_stale_volume_cleanup(
        "default",
        "claim-1",
        Err(ReconcileError::FieldMissing(
            "v1.PersistentVolumeClaim".to_string(),
            "metadata.name".to_string(),
        )),
    );

    assert_eq!(result, None);
}

#[test]
fn strict_cleanup_requires_controller_publish_secrets() {
    let volume = PublishedVolume {
        claim_name: "data".to_string(),
        driver: "example.csi".to_string(),
        volume_id: "volume-1".to_string(),
        target_path: "/var/lib/tugboat-agent/csi/ship-uid/data.fs".to_string(),
        access_type: PublishedAccessType::Filesystem,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: Some("/var/lib/tugboat-agent/csi/ship-uid/.staging/data".to_string()),
        controller_published: true,
        pvc_name: Some("data-pvc".to_string()),
    };

    let err = controller_publish_secrets_for_cleanup(
        &volume,
        &HashMap::new(),
        CleanupSecretPolicy::RequireControllerPublishSecrets,
    )
    .expect_err("active cleanup should reject missing controller publish secrets");

    assert!(err.contains("missing controller publish secrets"));
    assert!(err.contains("example.csi"));
    assert!(err.contains("/var/lib/tugboat-agent/csi/ship-uid/data.fs"));
}

#[test]
fn best_effort_cleanup_allows_missing_controller_publish_secrets() {
    let volume = PublishedVolume {
        claim_name: "data".to_string(),
        driver: "example.csi".to_string(),
        volume_id: "volume-1".to_string(),
        target_path: "/var/lib/tugboat-agent/csi/ship-uid/data.fs".to_string(),
        access_type: PublishedAccessType::Filesystem,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: Some("/var/lib/tugboat-agent/csi/ship-uid/.staging/data".to_string()),
        controller_published: true,
        pvc_name: Some("data-pvc".to_string()),
    };

    let secrets = controller_publish_secrets_for_cleanup(
        &volume,
        &HashMap::new(),
        CleanupSecretPolicy::BestEffort,
    )
    .expect("stale cleanup should allow missing controller publish secrets");

    assert!(secrets.is_empty());
}

#[test]
fn recovered_target_runtime_is_recreated_when_receiver_was_never_published_ready() {
    let ship = Ship {
        spec: Some(ShipSpec {
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_PENDING.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    assert_eq!(
        recovered_runtime_action(&ship, "node-2"),
        RecoveredRuntimeAction::RecreateTarget
    );
}

#[test]
fn recovered_failed_target_runtime_is_cleaned_up() {
    let ship = Ship {
        spec: Some(ShipSpec {
            target_node_name: Some("node-2".to_string()),
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

    assert_eq!(
        recovered_runtime_action(&ship, "node-2"),
        RecoveredRuntimeAction::CleanupFailedTarget
    );
}

#[test]
fn recovered_ready_target_runtime_is_registered() {
    let ship = Ship {
        spec: Some(ShipSpec {
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_READY.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    assert_eq!(
        recovered_runtime_action(&ship, "node-2"),
        RecoveredRuntimeAction::Register
    );
}

#[test]
fn recovered_completed_target_runtime_is_registered() {
    let ship = Ship {
        spec: Some(ShipSpec {
            target_node_name: Some("node-2".to_string()),
            ..Default::default()
        }),
        status: Some(ShipStatus {
            migration: Some(ShipMigrationStatus {
                phase: PHASE_COMPLETED.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    assert_eq!(
        recovered_runtime_action(&ship, "node-2"),
        RecoveredRuntimeAction::Register
    );
}

#[test]
fn recovered_volume_lookup_prefers_claim_name_match() {
    let claim_match = PublishedVolume {
        claim_name: "data".to_string(),
        driver: "example.csi".to_string(),
        volume_id: "volume-1".to_string(),
        target_path: "/var/lib/tugboat-agent/csi/ship-uid/data.fs".to_string(),
        access_type: PublishedAccessType::Filesystem,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: None,
        controller_published: true,
        pvc_name: Some("data-pvc".to_string()),
    };
    let id_match = PublishedVolume {
        claim_name: "legacy-data".to_string(),
        driver: "example.csi".to_string(),
        volume_id: "volume-1".to_string(),
        target_path: "/var/lib/tugboat-agent/csi/ship-uid/legacy-data.fs".to_string(),
        access_type: PublishedAccessType::Filesystem,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: None,
        controller_published: true,
        pvc_name: Some("data-pvc".to_string()),
    };

    let recovered = vec![claim_match.clone(), id_match];
    let found = find_recovered_published_volume(&recovered, "data", "volume-1")
        .expect("claim name match should be selected first");

    assert_eq!(found, &claim_match);
}

#[test]
fn recovered_volume_lookup_falls_back_to_volume_id() {
    let recovered = vec![PublishedVolume {
        claim_name: "legacy-data".to_string(),
        driver: "example.csi".to_string(),
        volume_id: "volume-1".to_string(),
        target_path: "/var/lib/tugboat-agent/csi/ship-uid/legacy-data.fs".to_string(),
        access_type: PublishedAccessType::Filesystem,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: None,
        controller_published: true,
        pvc_name: Some("data-pvc".to_string()),
    }];

    let found = find_recovered_published_volume(&recovered, "data", "volume-1")
        .expect("volume id fallback should recover entry");

    assert_eq!(found.claim_name, "legacy-data");
}

fn test_volume(target_path: &str, volume_id: &str) -> PublishedVolume {
    PublishedVolume {
        claim_name: "data".to_string(),
        driver: "example.csi".to_string(),
        volume_id: volume_id.to_string(),
        target_path: target_path.to_string(),
        access_type: PublishedAccessType::Filesystem,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: Some(format!("{target_path}.staging")),
        controller_published: true,
        pvc_name: Some("data-pvc".to_string()),
    }
}

#[test]
fn recovered_validation_rejects_duplicate_volume_ids_in_planned_state() {
    let persisted = vec![test_volume(
        "/var/lib/tugboat-agent/csi/ship-uid/current.fs",
        "persisted-volume-id",
    )];
    let planned = vec![
        test_volume(
            "/var/lib/tugboat-agent/csi/ship-uid/new-a.fs",
            "shared-volume-id",
        ),
        test_volume(
            "/var/lib/tugboat-agent/csi/ship-uid/new-b.fs",
            "shared-volume-id",
        ),
    ];

    let err = validate_recovered_published_volumes("ship-uid", persisted, &planned)
        .expect_err("duplicate planned volume ids should fail fallback recovery");

    assert!(matches!(
        err,
        ReconcileError::RecoveredPublishedVolumeStateMismatch(ship)
        if ship == "ship-uid"
    ));
}

#[test]
fn recovered_validation_rejects_duplicate_volume_ids_in_persisted_state() {
    let persisted = vec![
        test_volume(
            "/var/lib/tugboat-agent/csi/ship-uid/old-a.fs",
            "shared-volume-id",
        ),
        test_volume(
            "/var/lib/tugboat-agent/csi/ship-uid/old-b.fs",
            "shared-volume-id",
        ),
    ];
    let planned = vec![test_volume(
        "/var/lib/tugboat-agent/csi/ship-uid/new.fs",
        "shared-volume-id",
    )];

    let err = validate_recovered_published_volumes("ship-uid", persisted, &planned)
        .expect_err("duplicate persisted volume ids should fail fallback recovery");

    assert!(matches!(
        err,
        ReconcileError::RecoveredPublishedVolumeStateMismatch(ship)
        if ship == "ship-uid"
    ));
}
