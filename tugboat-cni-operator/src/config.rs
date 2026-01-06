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

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct CniOperatorConfig {
    #[serde(default)]
    pub(crate) location: LocationConfig,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct LocationConfig {
    pub(crate) bin: String,
    pub(crate) config: String,
    pub(crate) netns: String,
}

impl Default for LocationConfig {
    fn default() -> Self {
        Self {
            bin: "/opt/cni/bin".to_string(),
            config: "/etc/cni/net.d".to_string(),
            netns: "/var/run/netns".to_string(),
        }
    }
}
