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

use crate::csi::PublishedVolume;
use crate::reconciler::error::ReconcileError;
use std::collections::HashMap;
use tracing::warn;
use tugboat_resources::manifests::core::v1::{Ship, ShipSpec};

use super::super::{PHASE_COMPLETED, PHASE_FAILED, PHASE_MIGRATING, PHASE_PENDING, PHASE_READY};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CleanupSecretPolicy {
    BestEffort,
    RequireControllerPublishSecrets,
}

pub(super) fn best_effort_stale_volume_cleanup<T>(
    namespace: &str,
    claim_name: &str,
    result: Result<T, ReconcileError>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(err) => {
            // The PVC or its referenced Secret may have already been deleted.
            // Continue without an entry so cleanup falls back to empty secrets.
            warn!(
                "Failed to resolve stale volume '{}' secrets in namespace '{}', \
                 proceeding with empty secrets for best-effort cleanup. \
                 ControllerUnpublishVolume may fail and manual CSI cleanup may be required: {}",
                claim_name, namespace, err
            );
            None
        }
    }
}

pub(super) fn runtime_fingerprints_for_ship(
    ship_spec: &ShipSpec,
    local_node_name: &str,
) -> Result<super::super::ShipFingerprints, ReconcileError> {
    if ship_spec.target_node_name.as_deref() == Some(local_node_name) {
        let mut migrated_spec = ship_spec.clone();
        migrated_spec.node_name = Some(local_node_name.to_string());
        migrated_spec.target_node_name = None;
        super::super::ShipFingerprints::new(&migrated_spec)
    } else {
        super::super::ShipFingerprints::new(ship_spec)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecoveredRuntimeAction {
    Register,
    RecreateTarget,
    CleanupFailedTarget,
}

pub(super) fn recovered_runtime_action(
    ship: &Ship,
    local_node_name: &str,
) -> RecoveredRuntimeAction {
    let Some(spec) = ship.spec.as_ref() else {
        return RecoveredRuntimeAction::Register;
    };
    if spec.target_node_name.as_deref() != Some(local_node_name) {
        return RecoveredRuntimeAction::Register;
    }

    match ship
        .status
        .as_ref()
        .and_then(|status| status.migration.as_ref())
        .map(|migration| migration.phase.as_str())
    {
        Some(PHASE_FAILED) => RecoveredRuntimeAction::CleanupFailedTarget,
        None | Some(PHASE_PENDING) => RecoveredRuntimeAction::RecreateTarget,
        Some(PHASE_READY | PHASE_MIGRATING | PHASE_COMPLETED) => RecoveredRuntimeAction::Register,
        Some(_) => RecoveredRuntimeAction::Register,
    }
}

pub(super) fn controller_publish_secrets_for_cleanup(
    volume: &PublishedVolume,
    controller_publish_secrets: &HashMap<String, HashMap<String, String>>,
    policy: CleanupSecretPolicy,
) -> Result<HashMap<String, String>, String> {
    match controller_publish_secrets.get(&volume.claim_name) {
        Some(secrets) => Ok(secrets.clone()),
        None if !volume.controller_published => Ok(HashMap::new()),
        None if matches!(policy, CleanupSecretPolicy::BestEffort) => {
            warn!(
                "Missing controller publish secrets for stale CSI volume alias '{}' \
                 (driver='{}', volume_id='{}'); cleanup will continue best-effort with empty \
                 secrets and may require manual detach",
                volume.claim_name, volume.driver, volume.volume_id
            );
            Ok(HashMap::new())
        }
        None => Err(format!(
            "missing controller publish secrets for claim alias '{}' \
             (driver='{}', volume_id='{}', target_path='{}')",
            volume.claim_name, volume.driver, volume.volume_id, volume.target_path
        )),
    }
}

pub(super) fn find_recovered_published_volume<'a>(
    recovered_published_volumes: &'a [PublishedVolume],
    claim_name: &str,
    volume_id: &str,
) -> Option<&'a PublishedVolume> {
    recovered_published_volumes
        .iter()
        .find(|published| published.claim_name == claim_name)
        .or_else(|| {
            recovered_published_volumes
                .iter()
                .find(|published| published.volume_id == volume_id)
        })
}
