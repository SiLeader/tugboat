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

    if persisted != planned_sorted {
        return Err(ReconcileError::RecoveredPublishedVolumeStateMismatch(
            ship_id.to_string(),
        ));
    }

    Ok(persisted)
}
