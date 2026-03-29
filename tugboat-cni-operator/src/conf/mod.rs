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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CniNetConf {
    #[serde(flatten)]
    pub header: CniConfHeader,
    #[serde(flatten)]
    pub content: CniConfContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CniNetConfList {
    #[serde(flatten)]
    pub header: CniConfHeader,
    pub plugins: Vec<CniConfContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CniConfHeader {
    pub cni_version: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum CniConfContent {
    Bridge {
        bridge: String,
        is_gateway: bool,
        #[serde(rename = "ipMasq")]
        ip_masquerade: bool,
        ipam: CniIpam,
    },
    Flannel {
        #[serde(skip_serializing_if = "Option::is_none")]
        subnet_file: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        data_dir: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        delegate: Option<CniFlannelDelegate>,
    },
    Portmap {
        capabilities: CniPortmapCapabilities,
    },
    Loopback,
}

impl CniConfContent {
    fn cni_type(&self) -> &'static str {
        match self {
            Self::Bridge { .. } => "bridge",
            Self::Flannel { .. } => "flannel",
            Self::Portmap { .. } => "portmap",
            Self::Loopback => "loopback",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CniFlannelDelegate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_gateway: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_default_gateway: Option<bool>,
    #[serde(rename = "ipMasq", skip_serializing_if = "Option::is_none")]
    pub ip_masquerade: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hairpin_mode: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CniPortmapCapabilities {
    pub port_mappings: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CniIpam {
    #[serde(rename = "type")]
    pub cni_type: String,
    pub subnet: String,
    pub routes: Vec<CniIpamRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CniIpamRoute {
    #[serde(rename = "dst")]
    pub destination: String,
}

pub trait CniNetworkConfiguration {
    fn name(&self) -> &str;
    fn entry_point(&self) -> Result<&str, crate::error::Error>;
    fn file_name(&self) -> String;

    fn serialize_to_file(&self, file: File) -> Result<(), crate::error::Error>;
}

impl CniNetworkConfiguration for CniNetConf {
    fn name(&self) -> &str {
        &self.header.name
    }

    fn entry_point(&self) -> Result<&str, crate::error::Error> {
        Ok(self.content.cni_type())
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

    fn entry_point(&self) -> Result<&str, crate::error::Error> {
        self.plugins
            .first()
            .map(CniConfContent::cni_type)
            .ok_or_else(|| {
                crate::error::Error::InvalidConfiguration(format!(
                    "CNI config '{}' must include at least one plugin",
                    self.name()
                ))
            })
    }

    fn file_name(&self) -> String {
        format!("10-{}.conflist", self.name())
    }

    fn serialize_to_file(&self, file: File) -> Result<(), crate::error::Error> {
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CniConfHeader, CniNetConfList, CniNetworkConfiguration};

    #[test]
    fn conflist_requires_at_least_one_plugin() {
        let conf = CniNetConfList {
            header: CniConfHeader {
                cni_version: "1.0.0".to_string(),
                name: "empty".to_string(),
            },
            plugins: vec![],
        };

        assert!(matches!(
            conf.entry_point(),
            Err(crate::error::Error::InvalidConfiguration(_))
        ));
    }
}
