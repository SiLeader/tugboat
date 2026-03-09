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
use tugboat_resources::manifests::core::v1::ShipNetworkClassReference;

#[derive(Debug)]
pub(crate) struct NetworkClassRefForError(ShipNetworkClassReference);

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
    #[error("Invalid NetworkClassRef: {0}")]
    InvalidNetworkClassRef(NetworkClassRefForError),
    #[error("NetworkClass '{0}' not found")]
    NetworkClassNotFound(NetworkClassRefForError),
    #[error("CNI error: {0}")]
    Cni(#[from] tugboat_cni_operator::Error),
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
