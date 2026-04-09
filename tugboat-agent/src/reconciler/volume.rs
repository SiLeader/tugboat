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

pub(crate) mod normalize;

use crate::csi::is_supported_access_mode;
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::{InvalidSecretVolumeDataError, ReconcileError};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
pub(crate) use normalize::{
    MaterializedFile, MaterializedVolumeSourceKind, NormalizedKeyToPath, NormalizedVolumeSource,
    build_materialized_files, normalized_ship_volumes,
};
use std::collections::{HashMap, HashSet};
use tugboat_client::Api;
use tugboat_resources::manifests::core::v1::{
    ConfigMap, CsiPersistentVolumeSource, PersistentVolume, PersistentVolumeClaim,
    PersistentVolumeClaimReference, PersistentVolumeClaimSpec, PersistentVolumeSpec,
    PersistentVolumeStatus, Secret, ShipSpec,
};

#[derive(Debug, Clone)]
pub(crate) enum VolumeInfo {
    PersistentVolumeClaim(Box<PersistentVolumeClaimVolumeInfo>),
    Materialized(MaterializedVolumeInfo),
}

#[derive(Debug, Clone)]
pub(crate) struct PersistentVolumeClaimVolumeInfo {
    pub name: String,
    pub claim_name: String,
    pub volume_name: String,
    pub claim: PersistentVolumeClaimSpec,
    pub volume: PersistentVolumeSpec,
    pub status: Option<PersistentVolumeStatus>,
    pub source: CsiPersistentVolumeSource,
}

#[derive(Debug, Clone)]
pub(crate) struct MaterializedVolumeInfo {
    pub name: String,
    pub files: Vec<MaterializedFile>,
}

impl VolumeInfo {
    pub(crate) fn persistent_volume_claim(&self) -> Option<&PersistentVolumeClaimVolumeInfo> {
        match self {
            Self::PersistentVolumeClaim(volume) => Some(volume.as_ref()),
            Self::Materialized(_) => None,
        }
    }

    pub(crate) fn materialized(&self) -> Option<&MaterializedVolumeInfo> {
        match self {
            Self::PersistentVolumeClaim(_) => None,
            Self::Materialized(volume) => Some(volume),
        }
    }
}

pub(crate) fn ship_references_materialized_resource(
    ship_spec: &ShipSpec,
    kind: MaterializedVolumeSourceKind,
    resource_name: &str,
) -> Result<bool, ReconcileError> {
    Ok(normalized_ship_volumes(ship_spec)?
        .into_iter()
        .any(|volume| match volume.source {
            NormalizedVolumeSource::ConfigMap { name, .. } => {
                kind == MaterializedVolumeSourceKind::ConfigMap && name == resource_name
            }
            NormalizedVolumeSource::Secret { secret_name, .. } => {
                kind == MaterializedVolumeSourceKind::Secret && secret_name == resource_name
            }
            NormalizedVolumeSource::PersistentVolumeClaim { .. } => false,
        }))
}

pub(crate) fn materialized_volume_names_for_resource(
    ship_spec: &ShipSpec,
    kind: MaterializedVolumeSourceKind,
    resource_name: &str,
) -> Result<Vec<String>, ReconcileError> {
    Ok(normalized_ship_volumes(ship_spec)?
        .into_iter()
        .filter_map(|volume| match volume.source {
            NormalizedVolumeSource::ConfigMap { name, .. }
                if kind == MaterializedVolumeSourceKind::ConfigMap && name == resource_name =>
            {
                Some(volume.name)
            }
            NormalizedVolumeSource::Secret { secret_name, .. }
                if kind == MaterializedVolumeSourceKind::Secret && secret_name == resource_name =>
            {
                Some(volume.name)
            }
            _ => None,
        })
        .collect())
}

impl ShipReconciler {
    pub(crate) async fn get_related_volumes(
        &self,
        namespace: &str,
        ship_spec: &ShipSpec,
    ) -> Result<Vec<VolumeInfo>, ReconcileError> {
        let claim_api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);
        let volume_api: Api<PersistentVolume> = Api::all(self.client.clone());
        let normalized = normalized_ship_volumes(ship_spec)?;
        let mut volumes = Vec::with_capacity(normalized.len());

        for volume in normalized {
            match volume.source {
                NormalizedVolumeSource::PersistentVolumeClaim { claim_name } => {
                    volumes.push(VolumeInfo::PersistentVolumeClaim(Box::new(
                        self.load_persistent_volume_claim(
                            namespace,
                            &claim_api,
                            &volume_api,
                            volume.name,
                            claim_name,
                        )
                        .await?,
                    )));
                }
                NormalizedVolumeSource::ConfigMap {
                    name,
                    items,
                    default_mode,
                    optional,
                } => {
                    volumes.push(VolumeInfo::Materialized(
                        self.load_config_map_volume(
                            namespace,
                            volume.name,
                            name,
                            items,
                            default_mode,
                            optional,
                        )
                        .await?,
                    ));
                }
                NormalizedVolumeSource::Secret {
                    secret_name,
                    items,
                    default_mode,
                    optional,
                } => {
                    volumes.push(VolumeInfo::Materialized(
                        self.load_secret_volume(
                            namespace,
                            volume.name,
                            secret_name,
                            items,
                            default_mode,
                            optional,
                        )
                        .await?,
                    ));
                }
            }
        }

        Ok(volumes)
    }

    pub(crate) async fn load_persistent_volume_claim(
        &self,
        namespace: &str,
        claim_api: &Api<PersistentVolumeClaim>,
        volume_api: &Api<PersistentVolume>,
        volume_name_alias: String,
        claim_name: String,
    ) -> Result<PersistentVolumeClaimVolumeInfo, ReconcileError> {
        let claim = match claim_api.get(&claim_name).await? {
            Some(claim) => claim,
            None => return Err(ReconcileError::PersistentVolumeClaimNotFound(claim_name)),
        };
        let claim_spec = claim
            .spec
            .ok_or_else(|| ReconcileError::PersistentVolumeClaimMissingSpec(claim_name.clone()))?;

        let persistent_volume_name = claim_spec
            .volume_name
            .clone()
            .ok_or_else(|| ReconcileError::PersistentVolumeClaimNotBound(claim_name.clone()))?;
        let volume = match volume_api.get(&persistent_volume_name).await? {
            Some(volume) => volume,
            None => {
                return Err(ReconcileError::PersistentVolumeNotFound(
                    persistent_volume_name.clone(),
                ));
            }
        };
        let volume_status = volume.status.clone();
        let volume_spec = volume.spec.ok_or_else(|| {
            ReconcileError::PersistentVolumeMissingSpec(persistent_volume_name.clone())
        })?;

        let claim_mode = effective_volume_mode(claim_spec.volume_mode.as_deref());
        let volume_mode = effective_volume_mode(volume_spec.volume_mode.as_deref());
        if claim_mode != volume_mode {
            return Err(ReconcileError::VolumeModeMismatch {
                claim: claim_name.clone(),
                claim_mode: claim_mode.to_string(),
                volume: persistent_volume_name.clone(),
                volume_mode: volume_mode.to_string(),
            });
        }
        ensure_supported_claim_mode(&claim_name, claim_mode)?;
        ensure_supported_persistent_volume_mode(&persistent_volume_name, volume_mode)?;
        ensure_volume_claim_binding(
            &persistent_volume_name,
            volume_spec.claim_ref.as_ref(),
            namespace,
            &claim_name,
        )?;
        ensure_access_modes_compatible(
            &claim_name,
            &claim_spec.access_modes,
            &persistent_volume_name,
            &volume_spec.access_modes,
        )?;

        let source = volume_spec.csi.clone().ok_or_else(|| {
            ReconcileError::PersistentVolumeMissingCsi(persistent_volume_name.clone())
        })?;
        ensure_supported_csi_source(&persistent_volume_name, &source)?;

        Ok(PersistentVolumeClaimVolumeInfo {
            name: volume_name_alias,
            claim_name,
            volume_name: persistent_volume_name,
            claim: claim_spec,
            volume: volume_spec,
            status: volume_status,
            source,
        })
    }

    async fn load_config_map_volume(
        &self,
        namespace: &str,
        volume_name: String,
        config_map_name: String,
        items: Vec<NormalizedKeyToPath>,
        default_mode: u32,
        optional: bool,
    ) -> Result<MaterializedVolumeInfo, ReconcileError> {
        let api: Api<ConfigMap> = Api::namespaced(self.client.clone(), namespace);
        let Some(config_map) = api.get(&config_map_name).await? else {
            if optional {
                return Ok(MaterializedVolumeInfo {
                    name: volume_name,
                    files: Vec::new(),
                });
            }
            return Err(ReconcileError::ConfigMapNotFound {
                volume: volume_name,
                namespace: namespace.to_string(),
                name: config_map_name,
            });
        };

        let data = config_map
            .data
            .into_iter()
            .map(|(key, value)| (key, value.into_bytes()))
            .collect::<HashMap<_, _>>();
        let files = build_materialized_files(
            &volume_name,
            MaterializedVolumeSourceKind::ConfigMap,
            &config_map_name,
            data,
            &items,
            default_mode,
            optional,
        )?;
        Ok(MaterializedVolumeInfo {
            name: volume_name,
            files,
        })
    }

    async fn load_secret_volume(
        &self,
        namespace: &str,
        volume_name: String,
        secret_name: String,
        items: Vec<NormalizedKeyToPath>,
        default_mode: u32,
        optional: bool,
    ) -> Result<MaterializedVolumeInfo, ReconcileError> {
        let api: Api<Secret> = Api::namespaced(self.client.clone(), namespace);
        let Some(secret) = api.get(&secret_name).await? else {
            if optional {
                return Ok(MaterializedVolumeInfo {
                    name: volume_name,
                    files: Vec::new(),
                });
            }
            return Err(ReconcileError::SecretVolumeNotFound {
                volume: volume_name,
                namespace: namespace.to_string(),
                name: secret_name,
            });
        };

        let files = build_materialized_files(
            &volume_name,
            MaterializedVolumeSourceKind::Secret,
            &secret_name,
            decode_secret_volume_data(&volume_name, namespace, &secret_name, secret)?,
            &items,
            default_mode,
            optional,
        )?;
        Ok(MaterializedVolumeInfo {
            name: volume_name,
            files,
        })
    }
}

pub(crate) fn effective_volume_mode(mode: Option<&str>) -> &str {
    mode.unwrap_or("Block")
}

pub(crate) fn ensure_volume_claim_binding(
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

pub(crate) fn ensure_access_modes_compatible(
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

pub(crate) fn ensure_supported_csi_source(
    _volume_name: &str,
    _source: &CsiPersistentVolumeSource,
) -> Result<(), ReconcileError> {
    Ok(())
}

pub(crate) fn ensure_supported_claim_mode(
    claim_name: &str,
    mode: &str,
) -> Result<(), ReconcileError> {
    if matches!(mode, "Block" | "Filesystem") {
        Ok(())
    } else {
        Err(ReconcileError::UnsupportedClaimVolumeMode {
            claim: claim_name.to_string(),
            mode: mode.to_string(),
        })
    }
}

pub(crate) fn ensure_supported_persistent_volume_mode(
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

pub(crate) fn decode_secret_volume_data(
    volume_name: &str,
    namespace: &str,
    secret_name: &str,
    secret: Secret,
) -> Result<HashMap<String, Vec<u8>>, ReconcileError> {
    let mut data = HashMap::new();
    for (key, value) in secret.data {
        let decoded = BASE64_STANDARD.decode(value).map_err(|err| {
            ReconcileError::InvalidSecretVolumeData(Box::new(InvalidSecretVolumeDataError {
                volume: volume_name.to_string(),
                namespace: namespace.to_string(),
                name: secret_name.to_string(),
                key: key.clone(),
                reason: err.to_string(),
            }))
        })?;
        data.insert(key, decoded);
    }
    for (key, value) in secret.string_data {
        data.insert(key, value.into_bytes());
    }
    Ok(data)
}

#[cfg(test)]
mod tests;
