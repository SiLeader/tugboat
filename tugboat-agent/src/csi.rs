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

use std::collections::HashMap;
use tugboat_csi_operator::{CsiAccessMode, CsiAccessType, TugboatCsiOperator};
use tugboat_resources::manifests::core::v1::{
    CsiPersistentVolumeSource, PersistentVolumeClaimSpec, PersistentVolumeSpec,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum CsiError {
    #[error("Driver error: {0}")]
    Driver(#[from] tugboat_csi_operator::Error),
    #[error("Driver '{0}' not found")]
    DriverNotFound(String),
    #[error("Unrecognize access mode: {0}")]
    UnrecognizeAccessMode(String),
    #[error("Unrecognize access type: {0}")]
    UnrecognizeAccessType(String),
    #[error("Access mode is missing in claim spec")]
    MissingAccessMode,
}

#[derive(Clone)]
pub(crate) struct CsiWrapper {
    operator: TugboatCsiOperator,
    drivers: CsiDrivers,
}

impl CsiWrapper {
    pub(crate) fn new(operator: TugboatCsiOperator, drivers: CsiDrivers) -> Self {
        Self { operator, drivers }
    }

    pub(crate) async fn publish(
        &self,
        volume_id: String,
        volume: PersistentVolumeSpec,
        claim: PersistentVolumeClaimSpec,
        source: CsiPersistentVolumeSource,
        target_directory: String,
    ) -> Result<(), CsiError> {
        let Some(uds_path) = self.drivers.get(&source.driver) else {
            return Err(CsiError::DriverNotFound(source.driver));
        };
        let access_mode = CsiAccessMode::try_convert_from_string(
            &claim
                .access_modes
                .get(0)
                .ok_or(CsiError::MissingAccessMode)?,
        )?;
        let access_type = CsiAccessType::try_convert_from_string(
            volume
                .volume_mode
                .as_ref()
                .map(|m| m.as_str())
                .unwrap_or("Block"),
        )?;
        self.operator
            .publish(
                &uds_path,
                volume_id,
                target_directory,
                source.read_only,
                access_mode,
                access_type,
            )
            .await?;
        Ok(())
    }
}

trait TryConvertFromString: Sized {
    fn try_convert_from_string(value: &str) -> Result<Self, CsiError>;
}

impl TryConvertFromString for CsiAccessMode {
    fn try_convert_from_string(value: &str) -> Result<Self, CsiError> {
        match value {
            "ReadOnlyMany" => Ok(CsiAccessMode::ReadOnlyMany),
            "ReadWriteOnce" => Ok(CsiAccessMode::ReadWriteOnce),
            "ReadWriteMany" => Ok(CsiAccessMode::ReadWriteMany),
            _ => Err(CsiError::UnrecognizeAccessMode(value.to_string())),
        }
    }
}

impl TryConvertFromString for CsiAccessType {
    fn try_convert_from_string(value: &str) -> Result<Self, CsiError> {
        match value {
            "Block" => Ok(CsiAccessType::Block),
            // "Filesystem" => Ok(CsiAccessType::Filesystem),
            _ => Err(CsiError::UnrecognizeAccessType(value.to_string())),
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct CsiDrivers {
    drivers: HashMap<String, String>,
}

impl CsiDrivers {
    pub fn add(mut self, driver: String, socket_path: String) -> Self {
        self.drivers.insert(driver, socket_path);
        self
    }

    pub fn get(&self, driver: &str) -> Option<&str> {
        self.drivers.get(driver).map(|s| s.as_str())
    }
}
