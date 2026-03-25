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

use crate::runtime::error::RuntimeError;
use std::fmt::{Display, Formatter};
use thiserror::Error;
use tugboat_resources::manifests::core::v1::{ShipNetworkClassReference, ShipVolumeClaimReference};

#[derive(Debug)]
pub(crate) struct NetworkClassRefForError(ShipNetworkClassReference);

#[derive(Debug)]
pub(crate) struct VolumeClaimRefForError(ShipVolumeClaimReference);

#[derive(Debug, Error)]
#[error(
    "Secret '{namespace}/{name}' referenced by PersistentVolume '{volume}' field '{field}' has invalid data for key '{key}': {reason}"
)]
pub(crate) struct InvalidCsiSecretDataError {
    pub(crate) volume: String,
    pub(crate) field: String,
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) key: String,
    pub(crate) reason: String,
}

#[derive(Debug, Error)]
pub(crate) enum ReconcileError {
    #[error("API error: {0}")]
    Api(#[from] tugboat_client::Error),
    #[error("Field '{1}' in '{0}' is missing")]
    FieldMissing(String, String),
    #[error("ShipClass '{0}' not found")]
    ShipClassNotFound(String),
    #[error("Runtime error: {0}")]
    Runtime(#[from] RuntimeError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid NetworkClassRef: {0}")]
    InvalidNetworkClassRef(NetworkClassRefForError),
    #[error("NetworkClass '{0}' not found")]
    NetworkClassNotFound(NetworkClassRefForError),
    #[error("Invalid VolumeClaimRef: {0}")]
    InvalidVolumeClaimRef(VolumeClaimRefForError),
    #[error("Duplicate VolumeClaimRef '{0}'")]
    DuplicateVolumeClaimRef(String),
    #[error("PersistentVolumeClaim '{0}' not found")]
    PersistentVolumeClaimNotFound(String),
    #[error("PersistentVolumeClaim '{0}' is not bound to a PersistentVolume")]
    PersistentVolumeClaimNotBound(String),
    #[error("PersistentVolumeClaim '{0}' has no spec")]
    PersistentVolumeClaimMissingSpec(String),
    #[error("PersistentVolume '{0}' not found")]
    PersistentVolumeNotFound(String),
    #[error("PersistentVolume '{0}' has no spec")]
    PersistentVolumeMissingSpec(String),
    #[error("PersistentVolume '{0}' is not bound to a PersistentVolumeClaim")]
    PersistentVolumeMissingClaimRef(String),
    #[error(
        "PersistentVolume '{volume}' is bound to PersistentVolumeClaim '{bound_namespace}/{bound_claim}', not '{claim_namespace}/{claim}'"
    )]
    PersistentVolumeClaimRefMismatch {
        volume: String,
        claim_namespace: String,
        claim: String,
        bound_namespace: String,
        bound_claim: String,
    },
    #[error("PersistentVolume '{0}' has no CSI source")]
    PersistentVolumeMissingCsi(String),
    #[error("PersistentVolumeClaim '{0}' has no access modes")]
    PersistentVolumeClaimMissingAccessModes(String),
    #[error("PersistentVolume '{0}' has no access modes")]
    PersistentVolumeMissingAccessModes(String),
    #[error("PersistentVolumeClaim '{claim}' uses unsupported access mode '{mode}'")]
    UnsupportedClaimAccessMode { claim: String, mode: String },
    #[error("PersistentVolume '{volume}' uses unsupported access mode '{mode}'")]
    UnsupportedPersistentVolumeAccessMode { volume: String, mode: String },
    #[error("PersistentVolumeClaim '{claim}' uses unsupported volume mode '{mode}'")]
    UnsupportedClaimVolumeMode { claim: String, mode: String },
    #[error("PersistentVolume '{volume}' uses unsupported volume mode '{mode}'")]
    UnsupportedPersistentVolumeMode { volume: String, mode: String },
    #[error(
        "PersistentVolumeClaim '{claim}' requests access modes '{claim_access_modes}', but PersistentVolume '{volume}' supports '{volume_access_modes}'"
    )]
    VolumeAccessModeMismatch {
        claim: String,
        claim_access_modes: String,
        volume: String,
        volume_access_modes: String,
    },
    #[error(
        "PersistentVolumeClaim '{claim}' requests volume mode '{claim_mode}', but PersistentVolume '{volume}' uses '{volume_mode}'"
    )]
    VolumeModeMismatch {
        claim: String,
        claim_mode: String,
        volume: String,
        volume_mode: String,
    },
    #[error(
        "Running ship '{0}' received a spec change that requires explicit recreate; live mutation is not supported"
    )]
    UnsupportedRunningShipModification(String),
    #[error("PersistentVolume '{volume}' uses unsupported CSI feature '{feature}'")]
    UnsupportedPersistentVolumeCsiFeature { volume: String, feature: String },
    #[error("PersistentVolume '{volume}' has an invalid CSI secret reference in field '{field}'")]
    InvalidCsiSecretReference { volume: String, field: String },
    #[error(
        "Secret '{namespace}/{name}' referenced by PersistentVolume '{volume}' field '{field}' was not found"
    )]
    CsiSecretNotFound {
        volume: String,
        field: String,
        namespace: String,
        name: String,
    },
    #[error(transparent)]
    InvalidCsiSecretData(Box<InvalidCsiSecretDataError>),
    #[error("CNI error: {0}")]
    Cni(#[from] tugboat_cni_operator::Error),
    #[error("CSI error: {0}")]
    Csi(#[from] crate::csi::CsiError),
}

impl Display for NetworkClassRefForError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}", self.0.api_group, self.0.kind, self.0.name)
    }
}

impl From<ShipNetworkClassReference> for NetworkClassRefForError {
    fn from(value: ShipNetworkClassReference) -> Self {
        Self(value)
    }
}

impl Display for VolumeClaimRefForError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.name)
    }
}

impl From<ShipVolumeClaimReference> for VolumeClaimRefForError {
    fn from(value: ShipVolumeClaimReference) -> Self {
        Self(value)
    }
}
