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
use crate::reconciler::error::{
    InvalidCsiSecretDataError, InvalidSecretVolumeDataError, ReconcileError,
};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tugboat_client::Api;
use tugboat_resources::manifests::core::v1::{
    ConfigMap, ConfigMapVolumeSource, CsiPersistentVolumeSource, KeyToPath, PersistentVolume,
    PersistentVolumeClaim, PersistentVolumeClaimReference, PersistentVolumeClaimSpec,
    PersistentVolumeClaimVolumeSource, PersistentVolumeSpec, PersistentVolumeStatus, Secret,
    SecretReference, SecretVolumeSource, ShipSpec, ShipVolume,
};

const DEFAULT_FILE_MODE: u32 = 0o644;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MaterializedFile {
    pub path: String,
    pub contents: Vec<u8>,
    pub mode: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaterializedVolumeSourceKind {
    ConfigMap,
    Secret,
}

impl MaterializedVolumeSourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ConfigMap => "ConfigMap",
            Self::Secret => "Secret",
        }
    }
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct NormalizedVolume {
    pub name: String,
    pub source: NormalizedVolumeSource,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) enum NormalizedVolumeSource {
    PersistentVolumeClaim {
        claim_name: String,
    },
    ConfigMap {
        name: String,
        items: Vec<NormalizedKeyToPath>,
        default_mode: u32,
        optional: bool,
    },
    Secret {
        secret_name: String,
        items: Vec<NormalizedKeyToPath>,
        default_mode: u32,
        optional: bool,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct NormalizedKeyToPath {
    pub key: String,
    pub path: String,
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

    async fn load_persistent_volume_claim(
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

impl ShipReconciler {
    pub(crate) async fn resolve_csi_secrets(
        &self,
        volume: &PersistentVolumeClaimVolumeInfo,
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
            mount_flags: volume
                .source
                .mount_options
                .iter()
                .filter(|value| !value.is_empty())
                .cloned()
                .collect(),
        })
    }

    async fn load_secret_reference(
        &self,
        volume_name: &str,
        field: &str,
        reference: Option<&SecretReference>,
    ) -> Result<HashMap<String, String>, ReconcileError> {
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

        decode_csi_secret_data(
            volume_name,
            field,
            &reference.namespace,
            &reference.name,
            secret,
        )
    }
}

pub(crate) fn normalized_ship_volumes(
    ship_spec: &ShipSpec,
) -> Result<Vec<NormalizedVolume>, ReconcileError> {
    let mut seen_legacy_claims = HashSet::new();
    let mut seen_names = HashSet::new();
    let mut volumes = Vec::new();

    for claim_ref in &ship_spec.volume_claim_ref {
        if claim_ref.name.is_empty() {
            return Err(ReconcileError::InvalidVolumeClaimRef(
                claim_ref.clone().into(),
            ));
        }
        if !seen_legacy_claims.insert(claim_ref.name.clone()) {
            return Err(ReconcileError::DuplicateVolumeClaimRef(
                claim_ref.name.clone(),
            ));
        }
        if !seen_names.insert(claim_ref.name.clone()) {
            return Err(ReconcileError::DuplicateShipVolume(claim_ref.name.clone()));
        }
        volumes.push(NormalizedVolume {
            name: claim_ref.name.clone(),
            source: NormalizedVolumeSource::PersistentVolumeClaim {
                claim_name: claim_ref.name.clone(),
            },
        });
    }

    for volume in &ship_spec.volumes {
        let normalized = normalize_ship_volume(volume)?;
        if !seen_names.insert(normalized.name.clone()) {
            return Err(ReconcileError::DuplicateShipVolume(normalized.name));
        }
        volumes.push(normalized);
    }

    Ok(volumes)
}

fn normalize_ship_volume(volume: &ShipVolume) -> Result<NormalizedVolume, ReconcileError> {
    if volume.name.is_empty() {
        return Err(ReconcileError::InvalidShipVolume {
            volume: "<unnamed>".to_string(),
            reason: "volume name must not be empty".to_string(),
        });
    }
    validate_materialized_volume_name(&volume.name)?;

    let source_count = usize::from(volume.persistent_volume_claim.is_some())
        + usize::from(volume.config_map.is_some())
        + usize::from(volume.secret.is_some());
    if source_count != 1 {
        return Err(ReconcileError::InvalidShipVolume {
            volume: volume.name.clone(),
            reason: "exactly one of persistentVolumeClaim, configMap, or secret must be set"
                .to_string(),
        });
    }

    let source = if let Some(source) = volume.persistent_volume_claim.as_ref() {
        normalize_persistent_volume_claim_source(&volume.name, source)?
    } else if let Some(source) = volume.config_map.as_ref() {
        normalize_config_map_source(&volume.name, source)?
    } else if let Some(source) = volume.secret.as_ref() {
        normalize_secret_source(&volume.name, source)?
    } else {
        unreachable!("volume source_count was validated above")
    };

    Ok(NormalizedVolume {
        name: volume.name.clone(),
        source,
    })
}

fn normalize_persistent_volume_claim_source(
    volume_name: &str,
    source: &PersistentVolumeClaimVolumeSource,
) -> Result<NormalizedVolumeSource, ReconcileError> {
    if source.claim_name.is_empty() {
        return Err(ReconcileError::InvalidShipVolume {
            volume: volume_name.to_string(),
            reason: "persistentVolumeClaim.claimName must not be empty".to_string(),
        });
    }
    Ok(NormalizedVolumeSource::PersistentVolumeClaim {
        claim_name: source.claim_name.clone(),
    })
}

fn normalize_config_map_source(
    volume_name: &str,
    source: &ConfigMapVolumeSource,
) -> Result<NormalizedVolumeSource, ReconcileError> {
    if source.name.is_empty() {
        return Err(ReconcileError::InvalidShipVolume {
            volume: volume_name.to_string(),
            reason: "configMap.name must not be empty".to_string(),
        });
    }
    Ok(NormalizedVolumeSource::ConfigMap {
        name: source.name.clone(),
        items: normalize_items(volume_name, &source.items)?,
        default_mode: normalize_default_mode(volume_name, source.default_mode)?,
        optional: source.optional.unwrap_or(false),
    })
}

fn normalize_secret_source(
    volume_name: &str,
    source: &SecretVolumeSource,
) -> Result<NormalizedVolumeSource, ReconcileError> {
    if source.secret_name.is_empty() {
        return Err(ReconcileError::InvalidShipVolume {
            volume: volume_name.to_string(),
            reason: "secret.secretName must not be empty".to_string(),
        });
    }
    Ok(NormalizedVolumeSource::Secret {
        secret_name: source.secret_name.clone(),
        items: normalize_items(volume_name, &source.items)?,
        default_mode: normalize_default_mode(volume_name, source.default_mode)?,
        optional: source.optional.unwrap_or(false),
    })
}

fn normalize_items(
    volume_name: &str,
    items: &[KeyToPath],
) -> Result<Vec<NormalizedKeyToPath>, ReconcileError> {
    let mut seen_paths = HashSet::new();
    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        if item.key.is_empty() {
            return Err(ReconcileError::InvalidShipVolume {
                volume: volume_name.to_string(),
                reason: "volume items[].key must not be empty".to_string(),
            });
        }
        validate_relative_target_path(volume_name, &item.path)?;
        if !seen_paths.insert(item.path.clone()) {
            return Err(ReconcileError::DuplicateVolumeItemPath {
                volume: volume_name.to_string(),
                path: item.path.clone(),
            });
        }
        normalized.push(NormalizedKeyToPath {
            key: item.key.clone(),
            path: item.path.clone(),
        });
    }
    Ok(normalized)
}

fn normalize_default_mode(volume_name: &str, mode: Option<i32>) -> Result<u32, ReconcileError> {
    let Some(mode) = mode else {
        return Ok(DEFAULT_FILE_MODE);
    };
    let mode = u32::try_from(mode).map_err(|_| ReconcileError::InvalidShipVolume {
        volume: volume_name.to_string(),
        reason: "defaultMode must be a positive integer".to_string(),
    })?;
    if mode > 0o777 {
        return Err(ReconcileError::InvalidShipVolume {
            volume: volume_name.to_string(),
            reason: "defaultMode must be between 0 and 0o777".to_string(),
        });
    }
    Ok(mode)
}

fn build_materialized_files(
    volume_name: &str,
    source_kind: MaterializedVolumeSourceKind,
    resource_name: &str,
    data: HashMap<String, Vec<u8>>,
    items: &[NormalizedKeyToPath],
    default_mode: u32,
    optional: bool,
) -> Result<Vec<MaterializedFile>, ReconcileError> {
    let mut files = Vec::new();
    let mut seen_paths = HashSet::new();

    if items.is_empty() {
        let mut entries = data.into_iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        for (key, contents) in entries {
            validate_relative_target_path(volume_name, &key)?;
            if !seen_paths.insert(key.clone()) {
                return Err(ReconcileError::DuplicateVolumeItemPath {
                    volume: volume_name.to_string(),
                    path: key,
                });
            }
            files.push(MaterializedFile {
                path: key,
                contents,
                mode: default_mode,
            });
        }
        return Ok(files);
    }

    for item in items {
        if !seen_paths.insert(item.path.clone()) {
            return Err(ReconcileError::DuplicateVolumeItemPath {
                volume: volume_name.to_string(),
                path: item.path.clone(),
            });
        }
        let Some(contents) = data.get(&item.key) else {
            if optional {
                continue;
            }
            return Err(ReconcileError::MissingVolumeItemKey {
                volume: volume_name.to_string(),
                kind: source_kind.as_str().to_string(),
                resource: resource_name.to_string(),
                key: item.key.clone(),
            });
        };
        files.push(MaterializedFile {
            path: item.path.clone(),
            contents: contents.clone(),
            mode: default_mode,
        });
    }

    Ok(files)
}

fn validate_relative_target_path(volume_name: &str, path: &str) -> Result<(), ReconcileError> {
    if path.is_empty() {
        return Err(ReconcileError::InvalidVolumeItemPath {
            volume: volume_name.to_string(),
            path: path.to_string(),
        });
    }
    let candidate = std::path::Path::new(path);
    if candidate.is_absolute() {
        return Err(ReconcileError::InvalidVolumeItemPath {
            volume: volume_name.to_string(),
            path: path.to_string(),
        });
    }
    if path
        .split('/')
        .any(|segment| matches!(segment, "" | "." | ".."))
    {
        return Err(ReconcileError::InvalidVolumeItemPath {
            volume: volume_name.to_string(),
            path: path.to_string(),
        });
    }

    for component in candidate.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(ReconcileError::InvalidVolumeItemPath {
                    volume: volume_name.to_string(),
                    path: path.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn validate_materialized_volume_name(volume_name: &str) -> Result<(), ReconcileError> {
    let candidate = std::path::Path::new(volume_name);
    if candidate.is_absolute() || volume_name.contains('/') {
        return Err(ReconcileError::InvalidShipVolume {
            volume: volume_name.to_string(),
            reason: "volume name must be a single relative path segment".to_string(),
        });
    }

    let mut components = candidate.components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(()),
        _ => Err(ReconcileError::InvalidShipVolume {
            volume: volume_name.to_string(),
            reason: "volume name must be a single relative path segment".to_string(),
        }),
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

fn decode_csi_secret_data(
    volume_name: &str,
    field: &str,
    namespace: &str,
    secret_name: &str,
    secret: Secret,
) -> Result<HashMap<String, String>, ReconcileError> {
    let mut data = HashMap::new();
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

fn decode_secret_volume_data(
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
        MaterializedFile, MaterializedVolumeSourceKind, build_materialized_files,
        decode_csi_secret_data, decode_secret_volume_data, effective_volume_mode,
        ensure_access_modes_compatible, ensure_supported_claim_mode, ensure_supported_csi_source,
        ensure_supported_persistent_volume_mode, ensure_volume_claim_binding,
        normalized_ship_volumes, validate_materialized_volume_name, validate_relative_target_path,
    };
    use tugboat_resources::manifests::core::v1::{
        ConfigMapVolumeSource, CsiPersistentVolumeSource, KeyToPath,
        PersistentVolumeClaimReference, PersistentVolumeClaimVolumeSource, Secret, SecretReference,
        SecretVolumeSource, ShipSpec, ShipVolume, ShipVolumeClaimReference,
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
    fn decodes_base64_csi_secret_data() {
        let secret = Secret {
            data: std::collections::HashMap::from([("token".to_string(), "c2VjcmV0".to_string())]),
            ..Default::default()
        };

        let decoded =
            decode_csi_secret_data("pv", "node_publish_secret_ref", "alpha", "publish", secret)
                .expect("secret decoding should succeed");
        assert_eq!(decoded.get("token"), Some(&"secret".to_string()));
    }

    #[test]
    fn decodes_binary_secret_volume_data() {
        let secret = Secret {
            data: std::collections::HashMap::from([("cert".to_string(), "AAE=".to_string())]),
            string_data: std::collections::HashMap::from([(
                "token".to_string(),
                "plain".to_string(),
            )]),
            ..Default::default()
        };

        let decoded = decode_secret_volume_data("secret-vol", "alpha", "app-secret", secret)
            .expect("secret decoding should succeed");
        assert_eq!(decoded.get("cert"), Some(&vec![0, 1]));
        assert_eq!(decoded.get("token"), Some(&b"plain".to_vec()));
    }

    #[test]
    fn normalizes_legacy_and_named_volumes() {
        let spec = ShipSpec {
            volume_claim_ref: vec![ShipVolumeClaimReference {
                name: "legacy-data".to_string(),
            }],
            volumes: vec![
                ShipVolume {
                    name: "cfg".to_string(),
                    config_map: Some(ConfigMapVolumeSource {
                        name: "app-config".to_string(),
                        items: vec![KeyToPath {
                            key: "app.toml".to_string(),
                            path: "config/app.toml".to_string(),
                        }],
                        default_mode: Some(0o640),
                        optional: Some(true),
                    }),
                    ..Default::default()
                },
                ShipVolume {
                    name: "secret".to_string(),
                    secret: Some(SecretVolumeSource {
                        secret_name: "app-secret".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ShipVolume {
                    name: "pvc".to_string(),
                    persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                        claim_name: "data".to_string(),
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let normalized = normalized_ship_volumes(&spec).expect("volume normalization must succeed");
        assert_eq!(normalized.len(), 4);
        assert_eq!(normalized[0].name, "legacy-data");
        assert_eq!(normalized[1].name, "cfg");
        assert_eq!(normalized[2].name, "secret");
        assert_eq!(normalized[3].name, "pvc");
    }

    #[test]
    fn rejects_multiple_named_volume_sources() {
        let spec = ShipSpec {
            volumes: vec![ShipVolume {
                name: "invalid".to_string(),
                config_map: Some(ConfigMapVolumeSource {
                    name: "cfg".to_string(),
                    ..Default::default()
                }),
                secret: Some(SecretVolumeSource {
                    secret_name: "secret".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(normalized_ship_volumes(&spec).is_err());
    }

    #[test]
    fn rejects_duplicate_named_volume_paths() {
        let err = normalized_ship_volumes(&ShipSpec {
            volumes: vec![ShipVolume {
                name: "cfg".to_string(),
                config_map: Some(ConfigMapVolumeSource {
                    name: "app-config".to_string(),
                    items: vec![
                        KeyToPath {
                            key: "a".to_string(),
                            path: "config/app".to_string(),
                        },
                        KeyToPath {
                            key: "b".to_string(),
                            path: "config/app".to_string(),
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap_err();

        assert!(format!("{err}").contains("duplicate target path"));
    }

    #[test]
    fn rejects_parent_dir_volume_paths() {
        assert!(validate_relative_target_path("cfg", "../etc/passwd").is_err());
        assert!(validate_relative_target_path("cfg", "/etc/passwd").is_err());
        assert!(validate_relative_target_path("cfg", "a/./b").is_err());
    }

    #[test]
    fn rejects_unsafe_volume_names() {
        assert!(validate_materialized_volume_name("../cfg").is_err());
        assert!(validate_materialized_volume_name("/etc/passwd").is_err());
        assert!(validate_materialized_volume_name("nested/cfg").is_err());
        assert!(validate_materialized_volume_name("cfg/").is_err());
        assert!(validate_materialized_volume_name("cfg").is_ok());
        assert!(validate_materialized_volume_name("cfg.v1").is_ok());
    }

    #[test]
    fn rejects_named_volumes_with_unsafe_names() {
        let err = normalized_ship_volumes(&ShipSpec {
            volumes: vec![ShipVolume {
                name: "../cfg".to_string(),
                config_map: Some(ConfigMapVolumeSource {
                    name: "app-config".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap_err();

        assert!(format!("{err}").contains("single relative path segment"));
    }

    #[test]
    fn materializes_all_keys_when_items_are_empty() {
        let mut files = build_materialized_files(
            "cfg",
            MaterializedVolumeSourceKind::ConfigMap,
            "app-config",
            std::collections::HashMap::from([
                ("app.toml".to_string(), b"[app]".to_vec()),
                ("log.toml".to_string(), b"[log]".to_vec()),
            ]),
            &[],
            0o640,
            false,
        )
        .expect("file projection should succeed");
        files.sort_by(|left, right| left.path.cmp(&right.path));

        assert_eq!(
            files,
            vec![
                MaterializedFile {
                    path: "app.toml".to_string(),
                    contents: b"[app]".to_vec(),
                    mode: 0o640,
                },
                MaterializedFile {
                    path: "log.toml".to_string(),
                    contents: b"[log]".to_vec(),
                    mode: 0o640,
                },
            ]
        );
    }

    #[test]
    fn projects_selected_keys_to_custom_paths() {
        let files = build_materialized_files(
            "cfg",
            MaterializedVolumeSourceKind::ConfigMap,
            "app-config",
            std::collections::HashMap::from([("app".to_string(), b"hello".to_vec())]),
            &[super::NormalizedKeyToPath {
                key: "app".to_string(),
                path: "config/app.txt".to_string(),
            }],
            0o600,
            false,
        )
        .expect("file projection should succeed");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "config/app.txt");
        assert_eq!(files[0].contents, b"hello".to_vec());
        assert_eq!(files[0].mode, 0o600);
    }
}
