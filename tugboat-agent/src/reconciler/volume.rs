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

use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use std::collections::HashSet;
use tugboat_client::Api;
use tugboat_resources::manifests::core::v1::{
    CsiPersistentVolumeSource, PersistentVolume, PersistentVolumeClaim, PersistentVolumeClaimSpec,
    PersistentVolumeSpec, ShipSpec,
};

#[derive(Debug, Clone)]
pub(crate) struct VolumeInfo {
    pub claim_name: String,
    pub claim: PersistentVolumeClaimSpec,
    pub volume: PersistentVolumeSpec,
    pub source: CsiPersistentVolumeSource,
}

impl ShipReconciler {
    pub(crate) async fn get_related_volumes(
        &self,
        namespace: &str,
        ship_spec: &ShipSpec,
    ) -> Result<Vec<VolumeInfo>, ReconcileError> {
        let claim_api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);
        let volume_api: Api<PersistentVolume> = Api::all(self.client.clone());
        let mut seen_claims = HashSet::new();
        let mut volumes = Vec::new();

        for claim_ref in &ship_spec.volume_claim_ref {
            if claim_ref.name.is_empty() {
                return Err(ReconcileError::InvalidVolumeClaimRef(
                    claim_ref.clone().into(),
                ));
            }
            if !seen_claims.insert(claim_ref.name.clone()) {
                return Err(ReconcileError::DuplicateVolumeClaimRef(
                    claim_ref.name.clone(),
                ));
            }

            let claim_name = claim_ref.name.clone();
            let claim = match claim_api.get(&claim_name).await? {
                Some(claim) => claim,
                None => return Err(ReconcileError::PersistentVolumeClaimNotFound(claim_name)),
            };
            let claim_spec = claim.spec.ok_or_else(|| {
                ReconcileError::PersistentVolumeClaimMissingSpec(claim_ref.name.clone())
            })?;

            let volume_name = claim_spec.volume_name.clone().ok_or_else(|| {
                ReconcileError::PersistentVolumeClaimNotBound(claim_ref.name.clone())
            })?;
            let volume = match volume_api.get(&volume_name).await? {
                Some(volume) => volume,
                None => {
                    return Err(ReconcileError::PersistentVolumeNotFound(
                        volume_name.clone(),
                    ));
                }
            };
            let volume_spec = volume
                .spec
                .ok_or_else(|| ReconcileError::PersistentVolumeMissingSpec(volume_name.clone()))?;

            let claim_mode = effective_volume_mode(claim_spec.volume_mode.as_deref());
            let volume_mode = effective_volume_mode(volume_spec.volume_mode.as_deref());
            if claim_mode != volume_mode {
                return Err(ReconcileError::VolumeModeMismatch {
                    claim: claim_ref.name.clone(),
                    claim_mode: claim_mode.to_string(),
                    volume: volume_name.clone(),
                    volume_mode: volume_mode.to_string(),
                });
            }
            ensure_block_claim_mode(&claim_ref.name, claim_mode)?;
            ensure_block_persistent_volume_mode(&volume_name, volume_mode)?;

            let source = volume_spec
                .csi
                .clone()
                .ok_or_else(|| ReconcileError::PersistentVolumeMissingCsi(volume_name.clone()))?;

            volumes.push(VolumeInfo {
                claim_name: claim_ref.name.clone(),
                claim: claim_spec,
                volume: volume_spec,
                source,
            });
        }

        Ok(volumes)
    }
}

fn effective_volume_mode(mode: Option<&str>) -> &str {
    mode.unwrap_or("Block")
}

fn ensure_block_claim_mode(claim_name: &str, mode: &str) -> Result<(), ReconcileError> {
    if mode == "Block" {
        Ok(())
    } else {
        Err(ReconcileError::UnsupportedClaimVolumeMode {
            claim: claim_name.to_string(),
            mode: mode.to_string(),
        })
    }
}

fn ensure_block_persistent_volume_mode(
    volume_name: &str,
    mode: &str,
) -> Result<(), ReconcileError> {
    if mode == "Block" {
        Ok(())
    } else {
        Err(ReconcileError::UnsupportedPersistentVolumeMode {
            volume: volume_name.to_string(),
            mode: mode.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        effective_volume_mode, ensure_block_claim_mode, ensure_block_persistent_volume_mode,
    };

    #[test]
    fn defaults_volume_mode_to_block() {
        assert_eq!(effective_volume_mode(None), "Block");
    }

    #[test]
    fn rejects_non_block_modes() {
        assert!(ensure_block_claim_mode("claim", "Filesystem").is_err());
        assert!(ensure_block_persistent_volume_mode("volume", "Filesystem").is_err());
    }
}
