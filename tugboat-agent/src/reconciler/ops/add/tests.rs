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

use super::{
    CSI_VOLUME_HEALTH_CONDITION, CSI_VOLUME_STATS_CONDITION,
    apply_persistent_volume_claim_csi_observation, apply_persistent_volume_csi_observation,
    find_recovered_published_volume,
    validate_recovered_published_volumes,
};
use crate::csi::{PublishedAccessType, PublishedVolume};
use crate::reconciler::error::ReconcileError;
use tugboat_csi_operator::{
    NodeVolumeStats, VolumeHealthCondition, VolumeUsageStats, VolumeUsageUnit,
};
use tugboat_resources::manifests::core::v1::{
    PersistentVolumeClaimCondition, PersistentVolumeCondition,
};
use tugboat_resources::manifests::meta::v1::Time;

fn published_volume(target_path: &str) -> PublishedVolume {
    PublishedVolume {
        claim_name: "data".to_string(),
        driver: "example.csi".to_string(),
        volume_id: format!("volume-{target_path}"),
        target_path: target_path.to_string(),
        access_type: PublishedAccessType::Filesystem,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: Some(format!("{target_path}.staging")),
        controller_published: false,
        pvc_name: Some("pvc-data".to_string()),
    }
}

#[test]
fn csi_observation_upserts_usage_and_health_conditions() {
    let stats = NodeVolumeStats {
        usage: vec![VolumeUsageStats {
            available: Some(3072),
            total: 4096,
            used: Some(1024),
            unit: VolumeUsageUnit::Bytes,
        }],
        condition: Some(VolumeHealthCondition {
            abnormal: true,
            message: "filesystem is read-only".to_string(),
        }),
    };
    let mut conditions = vec![PersistentVolumeCondition {
        status: "Existing".to_string(),
        message: "keep me".to_string(),
        timestamp: None,
    }];

    let changed = apply_persistent_volume_csi_observation(&mut conditions, &stats);

    assert!(changed);
    assert_eq!(conditions.len(), 3);
    assert!(conditions.iter().any(|condition| {
        condition.status == CSI_VOLUME_STATS_CONDITION
            && condition
                .message
                .contains("CSI driver reported volume usage: bytes(total=4096, used=1024, available=3072)")
            && condition.timestamp.is_some()
    }));
    assert!(conditions.iter().any(|condition| {
        condition.status == CSI_VOLUME_HEALTH_CONDITION
            && condition.message.contains(
                "CSI driver reported an abnormal volume condition: filesystem is read-only",
            )
            && condition.timestamp.is_some()
    }));
    assert!(
        conditions
            .iter()
            .any(|condition| condition.status == "Existing" && condition.message == "keep me")
    );
}

#[test]
fn csi_observation_removes_stale_claim_conditions_when_stats_disappear() {
    let stats = NodeVolumeStats {
        usage: Vec::new(),
        condition: None,
    };
    let mut conditions = vec![
        PersistentVolumeClaimCondition {
            status: CSI_VOLUME_STATS_CONDITION.to_string(),
            message: "old stats".to_string(),
            timestamp: None,
        },
        PersistentVolumeClaimCondition {
            status: CSI_VOLUME_HEALTH_CONDITION.to_string(),
            message: "old health".to_string(),
            timestamp: None,
        },
        PersistentVolumeClaimCondition {
            status: "Keep".to_string(),
            message: "keep me".to_string(),
            timestamp: None,
        },
    ];

    let changed = apply_persistent_volume_claim_csi_observation(&mut conditions, &stats);

    assert!(changed);
    assert_eq!(conditions.len(), 1);
    assert_eq!(conditions[0].status, "Keep");
    assert_eq!(conditions[0].message, "keep me");
}

#[test]
fn csi_observation_does_not_update_condition_when_message_is_unchanged() {
    let stats = NodeVolumeStats {
        usage: vec![VolumeUsageStats {
            available: Some(3072),
            total: 4096,
            used: Some(1024),
            unit: VolumeUsageUnit::Bytes,
        }],
        condition: None,
    };
    let timestamp = Some(Time::now());
    let mut conditions = vec![PersistentVolumeCondition {
        status: CSI_VOLUME_STATS_CONDITION.to_string(),
        message:
            "CSI driver reported volume usage: bytes(total=4096, used=1024, available=3072)"
                .to_string(),
        timestamp,
    }];

    let changed = apply_persistent_volume_csi_observation(&mut conditions, &stats);

    assert!(!changed);
    assert_eq!(conditions.len(), 1);
    assert_eq!(
        conditions[0].message,
        "CSI driver reported volume usage: bytes(total=4096, used=1024, available=3072)"
    );
    assert_eq!(conditions[0].timestamp, timestamp);
}

#[test]
fn recovered_volumes_require_persisted_state_when_volumes_exist() {
    let planned = vec![published_volume(
        "/var/lib/tugboat-agent/csi/ship-uid/data.fs",
    )];

    let err = validate_recovered_published_volumes("ship-uid", Vec::new(), &planned)
        .expect_err("missing persisted state should fail recovery");

    assert!(matches!(
        err,
        ReconcileError::MissingRecoveredPublishedVolumeState(ship)
        if ship == "ship-uid"
    ));
}

#[test]
fn recovered_volumes_reject_state_that_differs_from_plan() {
    let persisted = vec![published_volume(
        "/var/lib/tugboat-agent/csi/ship-uid/data.fs",
    )];
    let planned = vec![published_volume(
        "/var/lib/tugboat-agent/csi/ship-uid/other.fs",
    )];

    let err = validate_recovered_published_volumes("ship-uid", persisted, &planned)
        .expect_err("mismatched persisted state should fail recovery");

    assert!(matches!(
        err,
        ReconcileError::RecoveredPublishedVolumeStateMismatch(ship)
        if ship == "ship-uid"
    ));
}

#[test]
fn recovered_volumes_accept_matching_persisted_state() {
    let persisted = vec![published_volume(
        "/var/lib/tugboat-agent/csi/ship-uid/data.fs",
    )];

    let recovered =
        validate_recovered_published_volumes("ship-uid", persisted.clone(), &persisted)
            .expect("matching persisted state should be accepted");

    assert_eq!(recovered, persisted);
}

#[test]
fn recovered_volumes_accept_volume_id_match_with_different_metadata() {
    let mut persisted = published_volume("/var/lib/tugboat-agent/csi/ship-uid/old-data.fs");
    let planned = published_volume("/var/lib/tugboat-agent/csi/ship-uid/new-data.fs");
    persisted.volume_id = planned.volume_id.clone();
    persisted.claim_name = "legacy-alias".to_string();
    persisted.pvc_name = None;
    persisted.staging_target_path = None;

    let recovered = validate_recovered_published_volumes("ship-uid", vec![persisted.clone()], &[planned])
        .expect("volume_id match should be accepted for recovered state");

    assert_eq!(recovered, vec![persisted]);
}

#[test]
fn recovered_volumes_reject_duplicate_volume_ids() {
    let mut first = published_volume("/var/lib/tugboat-agent/csi/ship-uid/data-a.fs");
    let mut second = published_volume("/var/lib/tugboat-agent/csi/ship-uid/data-b.fs");
    second.volume_id = first.volume_id.clone();

    let err = validate_recovered_published_volumes("ship-uid", vec![first, second], &[])
        .expect_err("duplicate volume IDs should fail recovered-state matching");

    assert!(matches!(
        err,
        ReconcileError::RecoveredPublishedVolumeStateMismatch(ship)
        if ship == "ship-uid"
    ));
}

#[test]
fn find_recovered_volume_falls_back_to_volume_id() {
    let mut recovered = published_volume("/var/lib/tugboat-agent/csi/ship-uid/data.fs");
    recovered.claim_name = "legacy-alias".to_string();
    recovered.volume_id = "volume-stable-id".to_string();

    let recovered_volumes = vec![recovered.clone()];
    let found = find_recovered_published_volume(
        &recovered_volumes,
        "new-claim-name",
        &recovered.volume_id,
    )
    .expect("volume_id fallback lookup should find recovered volume");

    assert_eq!(found, &recovered);
}

