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
use std::path::Path;
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

pub fn load_toml_config<T: DeserializeOwned>(path: impl AsRef<Path>) -> crate::Result<T> {
    let path = path.as_ref();
    debug!("Loading TOML configuration from '{}'", path.display());
    let file = std::fs::read_to_string(path)?;
    toml::from_str(&file).map_err(crate::Error::from)
}

#[cfg(test)]
mod tests {
    use super::load_toml_config;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Config {
        runtime: RuntimeConfig,
    }

    #[derive(Debug, Deserialize)]
    struct RuntimeConfig {
        executable: String,
    }

    #[test]
    fn load_toml_config_reads_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.toml");
        std::fs::write(
            &path,
            r#"
[runtime]
executable = "/usr/bin/runtime"
"#,
        )
        .unwrap();

        let config: Config = load_toml_config(&path).unwrap();

        assert_eq!(config.runtime.executable, "/usr/bin/runtime");
    }
}
