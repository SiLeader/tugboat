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

use crate::csi::{PublishedAccessType, PublishedVolume};
use crate::reconciler::error::ReconcileError;
use crate::reconciler::volume::PersistentVolumeClaimVolumeInfo;
use tugboat_csi_operator::{
    NodeVolumeStats, VolumeHealthCondition, VolumeUsageStats, VolumeUsageUnit,
};
use tugboat_resources::manifests::core::v1::{
    PersistentVolumeClaimCondition, PersistentVolumeCondition,
};
use tugboat_resources::manifests::meta::v1::Time;
use tugboat_vm_runtime_interface::run::VmVolumeConfig;

pub(crate) const CSI_VOLUME_STATS_CONDITION: &str = "CsiVolumeStats";
pub(crate) const CSI_VOLUME_HEALTH_CONDITION: &str = "CsiVolumeHealth";

pub(crate) fn vm_volume_config(
    volume: &PersistentVolumeClaimVolumeInfo,
    published: &PublishedVolume,
    read_only: bool,
) -> VmVolumeConfig {
    match published.access_type {
        PublishedAccessType::Block => {
            VmVolumeConfig::block(published.target_path.clone(), "raw", read_only)
        }
        PublishedAccessType::Filesystem => VmVolumeConfig::filesystem(
            published.target_path.clone(),
            volume.name.clone(),
            read_only,
        ),
    }
}

pub(crate) fn apply_persistent_volume_csi_observation(
    conditions: &mut Vec<PersistentVolumeCondition>,
    stats: &NodeVolumeStats,
) -> bool {
    let mut changed = false;
    if let Some(message) = volume_usage_message(&stats.usage) {
        changed |=
            upsert_persistent_volume_condition(conditions, CSI_VOLUME_STATS_CONDITION, message);
    } else {
        changed |= remove_persistent_volume_condition(conditions, CSI_VOLUME_STATS_CONDITION);
    }
    if let Some(condition) = stats.condition.as_ref() {
        changed |= upsert_persistent_volume_condition(
            conditions,
            CSI_VOLUME_HEALTH_CONDITION,
            volume_health_message(condition),
        );
    } else {
        changed |= remove_persistent_volume_condition(conditions, CSI_VOLUME_HEALTH_CONDITION);
    }
    changed
}

pub(crate) fn apply_persistent_volume_claim_csi_observation(
    conditions: &mut Vec<PersistentVolumeClaimCondition>,
    stats: &NodeVolumeStats,
) -> bool {
    let mut changed = false;
    if let Some(message) = volume_usage_message(&stats.usage) {
        changed |= upsert_persistent_volume_claim_condition(
            conditions,
            CSI_VOLUME_STATS_CONDITION,
            message,
        );
    } else {
        changed |= remove_persistent_volume_claim_condition(conditions, CSI_VOLUME_STATS_CONDITION);
    }
    if let Some(condition) = stats.condition.as_ref() {
        changed |= upsert_persistent_volume_claim_condition(
            conditions,
            CSI_VOLUME_HEALTH_CONDITION,
            volume_health_message(condition),
        );
    } else {
        changed |=
            remove_persistent_volume_claim_condition(conditions, CSI_VOLUME_HEALTH_CONDITION);
    }
    changed
}

fn volume_usage_message(usage: &[VolumeUsageStats]) -> Option<String> {
    if usage.is_empty() {
        return None;
    }

    Some(format!(
        "CSI driver reported volume usage: {}",
        usage
            .iter()
            .map(volume_usage_summary)
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

fn volume_usage_summary(usage: &VolumeUsageStats) -> String {
    let unit = match usage.unit {
        VolumeUsageUnit::Bytes => "bytes",
        VolumeUsageUnit::Inodes => "inodes",
        VolumeUsageUnit::Unknown => "units",
    };
    let mut parts = vec![format!("total={}", usage.total)];
    if let Some(used) = usage.used {
        parts.push(format!("used={used}"));
    }
    if let Some(available) = usage.available {
        parts.push(format!("available={available}"));
    }
    format!("{unit}({})", parts.join(", "))
}

fn volume_health_message(condition: &VolumeHealthCondition) -> String {
    match (condition.abnormal, condition.message.trim()) {
        (true, "") => "CSI driver reported an abnormal volume condition.".to_string(),
        (true, message) => {
            format!("CSI driver reported an abnormal volume condition: {message}")
        }
        (false, "") => "CSI driver reports the volume is healthy.".to_string(),
        (false, message) => format!("CSI driver reports the volume is healthy: {message}"),
    }
}

pub(crate) fn upsert_persistent_volume_condition(
    conditions: &mut Vec<PersistentVolumeCondition>,
    status: &str,
    message: String,
) -> bool {
    upsert_condition(
        conditions,
        status,
        message,
        |condition| &condition.status,
        |condition| &mut condition.message,
        |condition| &mut condition.timestamp,
    )
}

pub(crate) fn remove_persistent_volume_condition(
    conditions: &mut Vec<PersistentVolumeCondition>,
    status: &str,
) -> bool {
    remove_condition(conditions, status, |condition| &condition.status)
}

pub(crate) fn upsert_persistent_volume_claim_condition(
    conditions: &mut Vec<PersistentVolumeClaimCondition>,
    status: &str,
    message: String,
) -> bool {
    upsert_condition(
        conditions,
        status,
        message,
        |condition| &condition.status,
        |condition| &mut condition.message,
        |condition| &mut condition.timestamp,
    )
}

pub(crate) fn remove_persistent_volume_claim_condition(
    conditions: &mut Vec<PersistentVolumeClaimCondition>,
    status: &str,
) -> bool {
    remove_condition(conditions, status, |condition| &condition.status)
}

fn upsert_condition<T, FStatus, FMessage, FTimestamp>(
    conditions: &mut Vec<T>,
    status: &str,
    message: String,
    status_ref: FStatus,
    message_ref: FMessage,
    timestamp_ref: FTimestamp,
) -> bool
where
    T: Default + ConditionStatus,
    FStatus: Fn(&T) -> &String,
    FMessage: Fn(&mut T) -> &mut String,
    FTimestamp: Fn(&mut T) -> &mut Option<Time>,
{
    if let Some(existing) = conditions
        .iter_mut()
        .find(|condition| status_ref(condition) == status)
    {
        let existing_message = message_ref(existing);
        if existing_message.as_str() == message.as_str() {
            return false;
        }
        *existing_message = message;
        *timestamp_ref(existing) = Some(Time::now());
        return true;
    }

    let mut condition = T::default();
    *condition.status_mut() = status.to_string();
    *message_ref(&mut condition) = message;
    *timestamp_ref(&mut condition) = Some(Time::now());
    conditions.push(condition);
    true
}

fn remove_condition<T, FStatus>(conditions: &mut Vec<T>, status: &str, status_ref: FStatus) -> bool
where
    FStatus: Fn(&T) -> &String,
{
    let original_len = conditions.len();
    conditions.retain(|condition| status_ref(condition) != status);
    original_len != conditions.len()
}

pub(crate) trait ConditionStatus {
    fn status_mut(&mut self) -> &mut String;
}

impl ConditionStatus for PersistentVolumeCondition {
    fn status_mut(&mut self) -> &mut String {
        &mut self.status
    }
}

impl ConditionStatus for PersistentVolumeClaimCondition {
    fn status_mut(&mut self) -> &mut String {
        &mut self.status
    }
}

pub(crate) fn validate_recovered_published_volumes(
    ship_id: &str,
    mut persisted: Vec<PublishedVolume>,
    planned: &[PublishedVolume],
) -> Result<Vec<PublishedVolume>, ReconcileError> {
    if persisted.is_empty() {
        if planned.is_empty() {
            return Ok(persisted);
        }
        return Err(ReconcileError::MissingRecoveredPublishedVolumeState(
            ship_id.to_string(),
        ));
    }

    persisted.sort_by(|left, right| left.target_path.cmp(&right.target_path));
    let mut planned_sorted = planned.to_vec();
    planned_sorted.sort_by(|left, right| left.target_path.cmp(&right.target_path));

    // Exact match: volumes fully published and recovered
    if persisted == planned_sorted {
        return Ok(persisted);
    }

    // Fallback recovery by volume_id is ambiguous when duplicates exist.
    // In that case, fail fast and let reconcile trigger explicit cleanup.
    if has_duplicate_volume_ids(&planned_sorted) || has_duplicate_volume_ids(&persisted) {
        return Err(ReconcileError::RecoveredPublishedVolumeStateMismatch(
            ship_id.to_string(),
        ));
    }

    // Backward-compatible recovery: match planned volumes by volume_id even if
    // persisted metadata (paths/aliases/optional fields) differs.
    let persisted_by_volume_id = persisted
        .into_iter()
        .map(|volume| (volume.volume_id.clone(), volume))
        .collect::<std::collections::HashMap<_, _>>();
    let mut recovered_by_id = Vec::with_capacity(planned_sorted.len());
    for planned_volume in &planned_sorted {
        let Some(recovered) = persisted_by_volume_id.get(&planned_volume.volume_id) else {
            return Err(ReconcileError::RecoveredPublishedVolumeStateMismatch(
                ship_id.to_string(),
            ));
        };
        recovered_by_id.push(recovered.clone());
    }
    recovered_by_id.sort_by(|left, right| left.target_path.cmp(&right.target_path));
    Ok(recovered_by_id)
}

fn has_duplicate_volume_ids(volumes: &[PublishedVolume]) -> bool {
    let mut seen = std::collections::HashSet::with_capacity(volumes.len());
    for volume in volumes {
        if !seen.insert(volume.volume_id.as_str()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{
        CSI_VOLUME_HEALTH_CONDITION, CSI_VOLUME_STATS_CONDITION,
        apply_persistent_volume_claim_csi_observation, apply_persistent_volume_csi_observation,
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
                && condition.message.contains(
                    "CSI driver reported volume usage: bytes(total=4096, used=1024, available=3072)",
                )
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

        let recovered =
            validate_recovered_published_volumes("ship-uid", vec![persisted.clone()], &[planned])
                .expect("volume_id match should be accepted for recovered state");

        assert_eq!(recovered, vec![persisted]);
    }

    #[test]
    fn recovered_volumes_reject_duplicate_volume_ids() {
        let first = published_volume("/var/lib/tugboat-agent/csi/ship-uid/data-a.fs");
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
}
