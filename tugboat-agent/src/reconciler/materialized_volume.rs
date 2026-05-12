use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::volume::{
    MaterializedVolumeInfo, MaterializedVolumeSourceKind, ProjectedServiceAccountTokenInfo,
    materialized_volume_names_for_resource,
};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error};
use tugboat_client::{BoundObjectReference, ServiceAccountTokenRequest};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::Ship;

impl ShipReconciler {
    pub(crate) fn materialize_volume(
        &self,
        ship_id: &str,
        volume: &MaterializedVolumeInfo,
    ) -> Result<String, ReconcileError> {
        let volume_dir = self.materialized_volume_dir(ship_id, &volume.name);
        if !volume_dir.exists() {
            create_dir_with_mode(&volume_dir, 0o700)
                .map_err(|err| materialized_io_error(&volume.name, &volume_dir, err.to_string()))?;
        }

        let mut desired_paths = HashSet::new();
        for file in &volume.files {
            let path = volume_dir.join(&file.path);
            let Some(parent) = path.parent() else {
                return Err(materialized_io_error(
                    &volume.name,
                    &path,
                    "projected file has no parent directory".to_string(),
                ));
            };
            create_dir_with_mode(parent, 0o700)
                .map_err(|err| materialized_io_error(&volume.name, parent, err.to_string()))?;
            write_file_atomically(&path, &file.contents, file.mode)
                .map_err(|err| materialized_io_error(&volume.name, &path, err.to_string()))?;
            desired_paths.insert(path);
        }

        // Cleanup files no longer in the spec
        self.cleanup_stale_files(&volume_dir, &desired_paths)
            .map_err(|err| materialized_io_error(&volume.name, &volume_dir, err.to_string()))?;

        Ok(volume_dir.display().to_string())
    }

    fn cleanup_stale_files(&self, dir: &Path, desired_paths: &HashSet<PathBuf>) -> io::Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                self.cleanup_stale_files(&path, desired_paths)?;
                // Remove directory if empty
                if fs::read_dir(&path)?.next().is_none() {
                    fs::remove_dir(&path)?;
                }
            } else if !desired_paths.contains(&path) {
                fs::remove_file(&path)?;
            }
        }
        Ok(())
    }

    pub(crate) fn cleanup_materialized_volumes(&self, ship_id: &str) -> Result<(), ReconcileError> {
        let root = self.materialized_ship_dir(ship_id);
        match fs::remove_dir_all(&root) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(materialized_io_error("<all>", &root, err.to_string())),
        }
    }

    pub(crate) fn clear_materialized_volume(
        &self,
        ship_id: &str,
        volume_name: &str,
    ) -> Result<(), ReconcileError> {
        let root = self.materialized_volume_dir(ship_id, volume_name);
        clear_directory_contents(&root)
            .map_err(|err| materialized_io_error(volume_name, &root, err.to_string()))
    }

    pub(crate) fn clear_materialized_volumes_for_dependency(
        &self,
        ship: &Ship,
        kind: MaterializedVolumeSourceKind,
        resource_name: &str,
    ) -> Result<(), ReconcileError> {
        let Some(metadata) = ship.object_meta() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata".to_string(),
            ));
        };
        let Some(ship_id) = metadata.uid.as_deref() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "metadata.uid".to_string(),
            ));
        };
        let Some(ship_spec) = ship.spec.as_ref() else {
            return Err(ReconcileError::FieldMissing(
                "v1.Ship".to_string(),
                "spec".to_string(),
            ));
        };

        for volume_name in materialized_volume_names_for_resource(ship_spec, kind, resource_name)? {
            self.clear_materialized_volume(ship_id, &volume_name)?;
        }
        Ok(())
    }

    fn materialized_ship_dir(&self, ship_id: &str) -> PathBuf {
        self.volume_data_dir.join(ship_id).join(".materialized")
    }

    fn materialized_volume_dir(&self, ship_id: &str, volume_name: &str) -> PathBuf {
        self.materialized_ship_dir(ship_id).join(volume_name)
    }

    pub(crate) fn start_service_account_token_refresh(
        &self,
        ship_id: &str,
        namespace: &str,
        volume: &MaterializedVolumeInfo,
    ) {
        for token in &volume.service_account_tokens {
            self.spawn_service_account_token_refresh(
                ship_id.to_string(),
                namespace.to_string(),
                volume.name.clone(),
                token.clone(),
            );
        }
    }

    fn spawn_service_account_token_refresh(
        &self,
        ship_id: String,
        namespace: String,
        volume_name: String,
        token: ProjectedServiceAccountTokenInfo,
    ) {
        let client = self.client.clone();
        let cancellation_token = self.cancellation_token.clone();
        let file_path = self
            .materialized_volume_dir(&ship_id, &volume_name)
            .join(&token.path);
        tokio::spawn(async move {
            let ttl = token.expiration_seconds.unwrap_or(3600).max(1);
            loop {
                let delay = refresh_delay(ttl);
                tokio::select! {
                    _ = sleep(delay) => {}
                    _ = cancellation_token.cancelled() => break,
                }

                if !file_path.exists() {
                    debug!(
                        "Stopping ServiceAccount token refresh for removed projected file '{}'",
                        file_path.display()
                    );
                    break;
                }

                match client
                    .create_service_account_token(
                        &namespace,
                        &token.service_account_name,
                        ServiceAccountTokenRequest {
                            audiences: token.audience.clone().into_iter().collect(),
                            expiration_seconds: token.expiration_seconds,
                            bound_object_ref: Some(BoundObjectReference {
                                kind: "Ship".to_string(),
                                api_version: "v1".to_string(),
                                name: token.ship_name.clone(),
                                uid: Some(token.ship_uid.clone()),
                            }),
                        },
                    )
                    .await
                {
                    Ok(response) => {
                        if let Err(err) =
                            write_file_atomically(&file_path, response.token.as_bytes(), 0o600)
                        {
                            error!(
                                "Failed to refresh projected ServiceAccount token at '{}': {err}",
                                file_path.display()
                            );
                        }
                    }
                    Err(err) => {
                        error!(
                            "Failed to refresh projected ServiceAccount token for '{}/{}': {err}",
                            namespace, token.service_account_name
                        );
                    }
                }
            }
        });
    }
}

fn refresh_delay(ttl_seconds: u64) -> Duration {
    let leeway = (ttl_seconds / 5).max(1);
    Duration::from_secs(ttl_seconds.saturating_sub(leeway).max(1))
}

fn create_dir_with_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn write_file_atomically(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("file has no parent directory"))?;
    let tmp_path = parent.join(format!(
        ".{}.tugboat-tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));

    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }

    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn clear_directory_contents(path: &Path) -> io::Result<()> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };

    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn materialized_io_error(volume: &str, path: &Path, reason: String) -> ReconcileError {
    ReconcileError::MaterializedVolumeIo {
        volume: volume.to_string(),
        path: path.display().to_string(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::{clear_directory_contents, create_dir_with_mode, write_file_atomically};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn creates_directory_with_requested_mode() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "tugboat-agent-materialized-volume-test-{}-{unique}",
            std::process::id()
        ));
        let path = root.join("nested").join("dir");
        create_dir_with_mode(&path, 0o750).expect("directory creation should succeed");

        let metadata = fs::metadata(&path).expect("metadata should exist");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o750);
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn creates_file_with_requested_mode_atomically() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "tugboat-agent-materialized-file-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temporary directory should be created");
        let path = root.join("secret.txt");

        write_file_atomically(&path, b"top-secret", 0o600).expect("file creation should succeed");

        let metadata = fs::metadata(&path).expect("metadata should exist");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::read_to_string(&path).expect("file should be readable"),
            "top-secret"
        );
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn clears_directory_contents_but_keeps_volume_root() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "tugboat-agent-materialized-clear-test-{}-{unique}",
            std::process::id()
        ));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("nested directory should be created");
        fs::write(root.join("config.toml"), "key = 'value'").expect("file should be written");
        fs::write(nested.join("secret.txt"), "top-secret").expect("nested file should be written");

        clear_directory_contents(&root).expect("directory contents should be cleared");

        assert!(root.exists());
        assert!(
            fs::read_dir(&root)
                .expect("volume root should remain readable")
                .next()
                .is_none()
        );
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }
}
