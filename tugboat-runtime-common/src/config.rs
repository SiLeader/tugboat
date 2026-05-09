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
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::debug;

#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("failed to read {component} config '{path}': {source}")]
    Read {
        component: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {component} config '{path}' as TOML: {source}")]
    ParseToml {
        component: &'static str,
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
}

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

pub fn load_component_toml_config<T: DeserializeOwned>(
    component: &'static str,
    path: impl AsRef<Path>,
) -> Result<T, ConfigLoadError> {
    let path = path.as_ref();
    debug!(
        component,
        path = %path.display(),
        "Loading component TOML configuration"
    );
    let content = std::fs::read_to_string(path).map_err(|source| ConfigLoadError::Read {
        component,
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&content).map_err(|source| ConfigLoadError::ParseToml {
        component,
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

#[cfg(test)]
mod tests {
    use super::{ConfigLoadError, load_component_toml_config, load_toml_config};
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

    #[test]
    fn component_toml_config_error_includes_component_path_and_read_cause() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");

        let err = load_component_toml_config::<Config>("tugboat-test", &path).unwrap_err();

        match &err {
            ConfigLoadError::Read {
                component,
                path: err_path,
                ..
            } => {
                assert_eq!(*component, "tugboat-test");
                assert_eq!(err_path, &path);
            }
            other => panic!("expected read error, got {other:?}"),
        }
        let message = err.to_string();
        assert!(message.contains("failed to read tugboat-test config"));
        assert!(message.contains(path.to_str().unwrap()));
    }

    #[test]
    fn component_toml_config_error_includes_component_path_and_parse_cause() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.toml");
        std::fs::write(&path, "[runtime\n").unwrap();

        let err = load_component_toml_config::<Config>("tugboat-test", &path).unwrap_err();

        match &err {
            ConfigLoadError::ParseToml {
                component,
                path: err_path,
                ..
            } => {
                assert_eq!(*component, "tugboat-test");
                assert_eq!(err_path, &path);
            }
            other => panic!("expected TOML parse error, got {other:?}"),
        }
        let message = err.to_string();
        assert!(message.contains("failed to parse tugboat-test config"));
        assert!(message.contains(path.to_str().unwrap()));
    }
}
