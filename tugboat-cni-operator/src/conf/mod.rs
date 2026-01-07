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

use serde::{Deserialize, Serialize};
use std::fs::File;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CniNetConf {
    #[serde(flatten)]
    pub header: CniConfHeader,
    #[serde(flatten)]
    pub content: CniConfContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CniNetConfList {
    #[serde(flatten)]
    pub header: CniConfHeader,
    pub plugins: Vec<CniConfContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CniConfHeader {
    pub cni_version: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CniConfContent {
    #[serde(rename = "type")]
    pub cni_type: String,
    pub bridge: String,
    pub is_gateway: bool,
    #[serde(rename = "ipMasq")]
    pub ip_masquerade: bool,
    pub ipam: CniIpam,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CniIpam {
    #[serde(rename = "type")]
    pub cni_type: String,
    pub subnet: String,
    pub routes: Vec<CniIpamRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CniIpamRoute {
    #[serde(rename = "dst")]
    pub destination: String,
}

pub trait CniNetworkConfiguration {
    fn name(&self) -> &str;
    fn file_name(&self) -> String;

    fn serialize_to_file(&self, file: File) -> Result<(), crate::error::Error>;
}

impl CniNetworkConfiguration for CniNetConf {
    fn name(&self) -> &str {
        &self.header.name
    }

    fn file_name(&self) -> String {
        format!("10-{}.conf", self.name())
    }

    fn serialize_to_file(&self, file: File) -> Result<(), crate::error::Error> {
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }
}

impl CniNetworkConfiguration for CniNetConfList {
    fn name(&self) -> &str {
        &self.header.name
    }

    fn file_name(&self) -> String {
        format!("10-{}.conflist", self.name())
    }

    fn serialize_to_file(&self, file: File) -> Result<(), crate::error::Error> {
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }
}
