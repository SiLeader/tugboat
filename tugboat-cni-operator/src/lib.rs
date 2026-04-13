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
use std::path::Path;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

mod caller;
mod conf;
mod config;
mod error;

pub use conf::*;
pub use config::*;

#[derive(Debug, Clone)]
pub struct TugboatCniOperator {
    caller: caller::CniCaller,
    config_dir: PathBuf,
    netns: String,
}

impl TugboatCniOperator {
    pub fn new(config: CniOperatorConfig) -> Self {
        Self {
            caller: caller::CniCaller::new(&config.location.bin, &config.location.netns),
            config_dir: config.location.config.into(),
            netns: config.location.netns,
        }
    }

    pub async fn initialize(&self) -> Result<(), Error> {
        info!("Initialize CNI operator.");
        debug!("Creating config directory: {:?}", self.config_dir);
        tokio::fs::create_dir_all(&self.config_dir).await?;
        debug!("Creating netns directory: {:?}", self.netns);
        tokio::fs::create_dir_all(&self.netns).await?;
        Ok(())
    }

    pub async fn add(
        &self,
        container_id: &str,
        iface_name: &str,
        config: impl CniNetworkConfiguration,
    ) -> Result<(), Error> {
        let cni_type = config.entry_point()?.to_string();
        let config_file = self.config_dir.join(config.file_name());
        persist_config_file_atomically(&config_file, &config)?;
        self.caller
            .add(container_id, iface_name, &cni_type, config_file)
            .await
    }

    pub async fn del(
        &self,
        container_id: &str,
        iface_name: &str,
        config: impl CniNetworkConfiguration,
    ) -> Result<(), Error> {
        let cni_type = config.entry_point()?.to_string();
        let config_file = self.config_dir.join(config.file_name());
        if !config_file.try_exists()? {
            debug!(
                "CNI config file '{}' was missing for DEL; recreating",
                config_file.display()
            );
            persist_config_file_atomically(&config_file, &config)?;
        }
        self.caller
            .del(container_id, iface_name, &cni_type, &config_file)
            .await?;
        // Best-effort cleanup of the config file created by add().
        let _ = std::fs::remove_file(&config_file);
        Ok(())
    }
}

fn persist_config_file_atomically(
    config_file: &Path,
    config: &impl CniNetworkConfiguration,
) -> Result<(), Error> {
    let parent = config_file.parent().ok_or_else(|| {
        Error::InvalidConfiguration(format!(
            "CNI config file path '{}' has no parent directory",
            config_file.display()
        ))
    })?;
    std::fs::create_dir_all(parent)?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| Error::InvalidConfiguration(format!("system clock error: {err}")))?
        .as_nanos();
    let file_name = config_file.file_name().ok_or_else(|| {
        Error::InvalidConfiguration(format!(
            "CNI config file path '{}' has no file name",
            config_file.display()
        ))
    })?;
    let tmp_name = format!(
        ".{}.tmp-{}-{}",
        file_name.to_string_lossy(),
        std::process::id(),
        nanos
    );
    let tmp_file = parent.join(tmp_name);

    let write_result = (|| -> Result<(), Error> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_file)?;
        config.serialize_to_file(file)?;
        std::fs::rename(&tmp_file, config_file)?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp_file);
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::persist_config_file_atomically;
    use crate::{
        CniConfContent, CniConfHeader, CniNetConfList, CniNetworkConfiguration, TugboatCniOperator,
    };

    fn test_conflist() -> CniNetConfList {
        CniNetConfList {
            header: CniConfHeader {
                cni_version: "1.0.0".to_string(),
                name: "test-network".to_string(),
            },
            plugins: vec![CniConfContent::Loopback],
        }
    }

    fn unique_temp_dir() -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        path.push(format!("tugboat-cni-operator-test-{pid}-{nanos}"));
        path
    }

    #[test]
    fn persist_config_file_atomically_writes_valid_json() {
        let dir = unique_temp_dir();
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        let config_file = dir.join(test_conflist().file_name());

        persist_config_file_atomically(&config_file, &test_conflist())
            .expect("atomic config write should succeed");

        let written = std::fs::read_to_string(&config_file).expect("config file should exist");
        assert!(written.contains("\"plugins\""));

        std::fs::remove_dir_all(&dir).expect("temp dir should be removed");
    }

    #[tokio::test]
    async fn del_recreates_missing_config_file_before_calling_plugin() {
        let dir = unique_temp_dir();
        let bin_dir = dir.join("bin");
        let config_dir = dir.join("configs");
        let netns_dir = dir.join("netns");
        std::fs::create_dir_all(&bin_dir).expect("bin dir should be created");

        // The fake plugin validates that stdin is valid JSON and exits success.
        let plugin_path = bin_dir.join("loopback");
        std::fs::write(&plugin_path, "#!/usr/bin/env sh\ncat >/dev/null\nexit 0\n")
            .expect("plugin script should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&plugin_path)
                .expect("plugin metadata should be available")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&plugin_path, perms)
                .expect("plugin script should be executable");
        }

        let operator = TugboatCniOperator::new(crate::CniOperatorConfig {
            location: crate::config::LocationConfig {
                bin: bin_dir.to_string_lossy().to_string(),
                config: config_dir.to_string_lossy().to_string(),
                netns: netns_dir.to_string_lossy().to_string(),
            },
        });
        operator
            .initialize()
            .await
            .expect("operator should initialize directories");

        let config = test_conflist();
        let config_file = config_dir.join(config.file_name());
        if config_file.exists() {
            std::fs::remove_file(&config_file).expect("stale config file should be removed");
        }

        operator
            .del("container-1", "lo", config)
            .await
            .expect("DEL should succeed even when config file was missing");

        assert!(!config_file.exists());
        std::fs::remove_dir_all(&dir).expect("temp dir should be removed");
    }
}
