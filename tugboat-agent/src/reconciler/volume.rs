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

use crate::csi::{ResolvedCsiSecrets, is_supported_access_mode};
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::{InvalidCsiSecretDataError, ReconcileError};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use std::collections::HashSet;
use tugboat_client::Api;
use tugboat_resources::manifests::core::v1::{
    CsiPersistentVolumeSource, PersistentVolume, PersistentVolumeClaim,
    PersistentVolumeClaimReference, PersistentVolumeClaimSpec, PersistentVolumeSpec,
    PersistentVolumeStatus, Secret, SecretReference, ShipSpec,
};

#[derive(Debug, Clone)]
pub(crate) struct VolumeInfo {
    pub claim_name: String,
    pub volume_name: String,
    pub claim: PersistentVolumeClaimSpec,
    pub volume: PersistentVolumeSpec,
    pub status: Option<PersistentVolumeStatus>,
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
            let volume_status = volume.status.clone();
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
            ensure_supported_claim_mode(&claim_ref.name, claim_mode)?;
            ensure_supported_persistent_volume_mode(&volume_name, volume_mode)?;
            ensure_volume_claim_binding(
                &volume_name,
                volume_spec.claim_ref.as_ref(),
                namespace,
                &claim_name,
            )?;
            ensure_access_modes_compatible(
                &claim_name,
                &claim_spec.access_modes,
                &volume_name,
                &volume_spec.access_modes,
            )?;

            let source = volume_spec
                .csi
                .clone()
                .ok_or_else(|| ReconcileError::PersistentVolumeMissingCsi(volume_name.clone()))?;
            ensure_supported_csi_source(&volume_name, &source)?;

            volumes.push(VolumeInfo {
                claim_name: claim_ref.name.clone(),
                volume_name: volume_name.clone(),
                claim: claim_spec,
                volume: volume_spec,
                status: volume_status,
                source,
            });
        }

        Ok(volumes)
    }
}

impl ShipReconciler {
    pub(crate) async fn resolve_csi_secrets(
        &self,
        volume: &VolumeInfo,
    ) -> Result<ResolvedCsiSecrets, ReconcileError> {
        Ok(ResolvedCsiSecrets {
            controller_publish: self
                .load_secret_reference(
                    &volume.volume_name,
                    "controller_publish_secret_ref",
                    volume.source.controller_publish_secret_ref.as_ref(),
                )
                .await?,
            node_expand: self
                .load_secret_reference(
                    &volume.volume_name,
                    "node_expand_secret_ref",
                    volume.source.node_expand_secret_ref.as_ref(),
                )
                .await?,
            node_publish: self
                .load_secret_reference(
                    &volume.volume_name,
                    "node_publish_secret_ref",
                    volume.source.node_publish_secret_ref.as_ref(),
                )
                .await?,
            node_stage: self
                .load_secret_reference(
                    &volume.volume_name,
                    "node_stage_secret_ref",
                    volume.source.node_stage_secret_ref.as_ref(),
                )
                .await?,
        })
    }

    async fn load_secret_reference(
        &self,
        volume_name: &str,
        field: &str,
        reference: Option<&SecretReference>,
    ) -> Result<std::collections::HashMap<String, String>, ReconcileError> {
        let Some(reference) = reference else {
            return Ok(Default::default());
        };
        if reference.name.is_empty() || reference.namespace.is_empty() {
            return Err(ReconcileError::InvalidCsiSecretReference {
                volume: volume_name.to_string(),
                field: field.to_string(),
            });
        }

        let api: Api<Secret> = Api::namespaced(self.client.clone(), &reference.namespace);
        let Some(secret) = api.get(&reference.name).await? else {
            return Err(ReconcileError::CsiSecretNotFound {
                volume: volume_name.to_string(),
                field: field.to_string(),
                namespace: reference.namespace.clone(),
                name: reference.name.clone(),
            });
        };

        decode_secret_data(
            volume_name,
            field,
            &reference.namespace,
            &reference.name,
            secret,
        )
    }
}

fn effective_volume_mode(mode: Option<&str>) -> &str {
    mode.unwrap_or("Block")
}

fn ensure_volume_claim_binding(
    volume_name: &str,
    claim_ref: Option<&PersistentVolumeClaimReference>,
    claim_namespace: &str,
    claim_name: &str,
) -> Result<(), ReconcileError> {
    match claim_ref {
        Some(claim_ref)
            if claim_ref.namespace == claim_namespace && claim_ref.name == claim_name =>
        {
            Ok(())
        }
        Some(claim_ref) => Err(ReconcileError::PersistentVolumeClaimRefMismatch {
            volume: volume_name.to_string(),
            claim_namespace: claim_namespace.to_string(),
            claim: claim_name.to_string(),
            bound_namespace: claim_ref.namespace.clone(),
            bound_claim: claim_ref.name.clone(),
        }),
        None => Err(ReconcileError::PersistentVolumeMissingClaimRef(
            volume_name.to_string(),
        )),
    }
}

fn ensure_access_modes_compatible(
    claim_name: &str,
    claim_access_modes: &[String],
    volume_name: &str,
    volume_access_modes: &[String],
) -> Result<(), ReconcileError> {
    if claim_access_modes.is_empty() {
        return Err(ReconcileError::PersistentVolumeClaimMissingAccessModes(
            claim_name.to_string(),
        ));
    }
    if volume_access_modes.is_empty() {
        return Err(ReconcileError::PersistentVolumeMissingAccessModes(
            volume_name.to_string(),
        ));
    }

    for mode in claim_access_modes {
        if !is_supported_access_mode(mode) {
            return Err(ReconcileError::UnsupportedClaimAccessMode {
                claim: claim_name.to_string(),
                mode: mode.clone(),
            });
        }
    }
    for mode in volume_access_modes {
        if !is_supported_access_mode(mode) {
            return Err(ReconcileError::UnsupportedPersistentVolumeAccessMode {
                volume: volume_name.to_string(),
                mode: mode.clone(),
            });
        }
    }

    let supported_modes = volume_access_modes
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if claim_access_modes
        .iter()
        .any(|mode| !supported_modes.contains(mode.as_str()))
    {
        return Err(ReconcileError::VolumeAccessModeMismatch {
            claim: claim_name.to_string(),
            claim_access_modes: claim_access_modes.join(", "),
            volume: volume_name.to_string(),
            volume_access_modes: volume_access_modes.join(", "),
        });
    }

    Ok(())
}

fn ensure_supported_csi_source(
    _volume_name: &str,
    _source: &CsiPersistentVolumeSource,
) -> Result<(), ReconcileError> {
    Ok(())
}

fn ensure_supported_claim_mode(claim_name: &str, mode: &str) -> Result<(), ReconcileError> {
    if matches!(mode, "Block" | "Filesystem") {
        Ok(())
    } else {
        Err(ReconcileError::UnsupportedClaimVolumeMode {
            claim: claim_name.to_string(),
            mode: mode.to_string(),
        })
    }
}

fn ensure_supported_persistent_volume_mode(
    volume_name: &str,
    mode: &str,
) -> Result<(), ReconcileError> {
    if matches!(mode, "Block" | "Filesystem") {
        Ok(())
    } else {
        Err(ReconcileError::UnsupportedPersistentVolumeMode {
            volume: volume_name.to_string(),
            mode: mode.to_string(),
        })
    }
}

fn decode_secret_data(
    volume_name: &str,
    field: &str,
    namespace: &str,
    secret_name: &str,
    secret: Secret,
) -> Result<std::collections::HashMap<String, String>, ReconcileError> {
    let mut data = std::collections::HashMap::new();
    for (key, value) in secret.data {
        let decoded = BASE64_STANDARD.decode(value).map_err(|err| {
            invalid_csi_secret_data(
                volume_name,
                field,
                namespace,
                secret_name,
                &key,
                err.to_string(),
            )
        })?;
        let decoded = String::from_utf8(decoded).map_err(|err| {
            invalid_csi_secret_data(
                volume_name,
                field,
                namespace,
                secret_name,
                &key,
                err.to_string(),
            )
        })?;
        data.insert(key, decoded);
    }
    data.extend(secret.string_data);
    Ok(data)
}

fn invalid_csi_secret_data(
    volume_name: &str,
    field: &str,
    namespace: &str,
    secret_name: &str,
    key: &str,
    reason: String,
) -> ReconcileError {
    ReconcileError::InvalidCsiSecretData(Box::new(InvalidCsiSecretDataError {
        volume: volume_name.to_string(),
        field: field.to_string(),
        namespace: namespace.to_string(),
        name: secret_name.to_string(),
        key: key.to_string(),
        reason,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        decode_secret_data, effective_volume_mode, ensure_access_modes_compatible,
        ensure_supported_claim_mode, ensure_supported_csi_source,
        ensure_supported_persistent_volume_mode, ensure_volume_claim_binding,
    };
    use tugboat_resources::manifests::core::v1::{
        CsiPersistentVolumeSource, PersistentVolumeClaimReference, Secret, SecretReference,
    };

    #[test]
    fn defaults_volume_mode_to_block() {
        assert_eq!(effective_volume_mode(None), "Block");
    }

    #[test]
    fn allows_supported_volume_modes() {
        assert!(ensure_supported_claim_mode("claim", "Filesystem").is_ok());
        assert!(ensure_supported_persistent_volume_mode("volume", "Filesystem").is_ok());
    }

    #[test]
    fn requires_volume_to_be_bound_to_exact_claim() {
        let claim_ref = PersistentVolumeClaimReference {
            name: "data".to_string(),
            namespace: "alpha".to_string(),
        };

        assert!(ensure_volume_claim_binding("pv-1", Some(&claim_ref), "alpha", "data").is_ok());
        assert!(ensure_volume_claim_binding("pv-1", Some(&claim_ref), "beta", "data").is_err());
        assert!(ensure_volume_claim_binding("pv-1", None, "alpha", "data").is_err());
    }

    #[test]
    fn rejects_access_mode_mismatches() {
        assert!(
            ensure_access_modes_compatible(
                "claim",
                &["ReadWriteMany".to_string()],
                "pv",
                &["ReadOnlyMany".to_string()],
            )
            .is_err()
        );
    }

    #[test]
    fn allows_controller_expand_secret_ref() {
        let source = CsiPersistentVolumeSource {
            controller_expand_secret_ref: Some(SecretReference {
                name: "expand-secret".to_string(),
                namespace: "alpha".to_string(),
            }),
            ..Default::default()
        };

        assert!(ensure_supported_csi_source("pv", &source).is_ok());
    }

    #[test]
    fn allows_node_publish_and_stage_secrets() {
        let source = CsiPersistentVolumeSource {
            node_publish_secret_ref: Some(SecretReference {
                name: "publish-secret".to_string(),
                namespace: "alpha".to_string(),
            }),
            node_stage_secret_ref: Some(SecretReference {
                name: "stage-secret".to_string(),
                namespace: "alpha".to_string(),
            }),
            fs_type: Some("xfs".to_string()),
            volume_attributes: std::collections::HashMap::from([(
                "storage.kubernetes.io/csiProvisionerIdentity".to_string(),
                "test".to_string(),
            )]),
            ..Default::default()
        };

        assert!(ensure_supported_csi_source("pv", &source).is_ok());
    }

    #[test]
    fn decodes_base64_secret_data() {
        let secret = Secret {
            data: std::collections::HashMap::from([("token".to_string(), "c2VjcmV0".to_string())]),
            ..Default::default()
        };

        let decoded =
            decode_secret_data("pv", "node_publish_secret_ref", "alpha", "publish", secret)
                .expect("secret decoding should succeed");
        assert_eq!(decoded.get("token"), Some(&"secret".to_string()));
    }
}
