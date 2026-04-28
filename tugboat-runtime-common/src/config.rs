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

use serde::de::DeserializeOwned;
use std::fs::File;
use tracing::debug;

pub fn load_config<T: DeserializeOwned>(path: String) -> crate::Result<T> {
    if path == "-" {
        debug!("Loading configuration from stdin");
        serde_json::from_reader(std::io::stdin()).map_err(crate::Error::from)
    } else {
        debug!("Loading configuration from '{path}'");
        let file = File::open(path)?;
        serde_json::from_reader(file).map_err(crate::Error::from)
    }
}
