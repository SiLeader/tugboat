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

pub use crate::error::Error;
use std::path::PathBuf;

mod bridge;
mod conf;
mod config;
mod error;
#[cfg(feature = "tap")]
mod tap;

pub use conf::*;
pub use config::*;

#[derive(Debug, Clone)]
pub struct CniOperator {
    bridge_caller: bridge::BridgeCaller,
    config_dir: PathBuf,
}

impl CniOperator {
    pub fn new(config: CniOperatorConfig) -> Self {
        Self {
            bridge_caller: bridge::BridgeCaller::new(&config.location.bin, &config.location.netns),
            config_dir: config.location.config.into(),
        }
    }

    pub async fn add(
        &self,
        container_id: &str,
        iface_name: &str,
        config: impl CniNetworkConfiguration,
    ) -> Result<(), Error> {
        let config_file = self.config_dir.join(config.file_name());
        config.serialize_to_file(std::fs::File::create(&config_file)?)?;
        self.bridge_caller
            .add(container_id, iface_name, config_file)
            .await
    }

    pub async fn del(
        &self,
        container_id: &str,
        iface_name: &str,
        config: impl CniNetworkConfiguration,
    ) -> Result<(), Error> {
        let config_file = self.config_dir.join(config.file_name());
        self.bridge_caller
            .del(container_id, iface_name, config_file)
            .await
    }
}
