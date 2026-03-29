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

use crate::reconciler::error::ReconcileError;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tugboat_resources::manifests::core::v1::{
    ConfigMapVolumeSource, KeyToPath, PersistentVolumeClaimVolumeSource, SecretVolumeSource,
    ShipSpec, ShipVolume,
};

pub(crate) const DEFAULT_FILE_MODE: u32 = 0o644;

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

pub(crate) fn normalize_ship_volume(
    volume: &ShipVolume,
) -> Result<NormalizedVolume, ReconcileError> {
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

pub(crate) fn build_materialized_files(
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

pub(crate) fn validate_relative_target_path(
    volume_name: &str,
    path: &str,
) -> Result<(), ReconcileError> {
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
        .any(|segment| matches!(segment, "." | ".."))
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

pub(crate) fn validate_materialized_volume_name(volume_name: &str) -> Result<(), ReconcileError> {
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
