use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::volume::MaterializedVolumeInfo;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

impl ShipReconciler {
    pub(crate) fn materialize_volume(
        &self,
        ship_id: &str,
        volume: &MaterializedVolumeInfo,
    ) -> Result<String, ReconcileError> {
        let volume_dir = self.materialized_volume_dir(ship_id, &volume.name);
        match fs::remove_dir_all(&volume_dir) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(materialized_io_error(
                    &volume.name,
                    &volume_dir,
                    err.to_string(),
                ));
            }
        }

        create_dir_with_mode(&volume_dir, 0o755)
            .map_err(|err| materialized_io_error(&volume.name, &volume_dir, err.to_string()))?;
        for file in &volume.files {
            let path = volume_dir.join(&file.path);
            let Some(parent) = path.parent() else {
                return Err(materialized_io_error(
                    &volume.name,
                    &path,
                    "projected file has no parent directory".to_string(),
                ));
            };
            create_dir_with_mode(parent, 0o755)
                .map_err(|err| materialized_io_error(&volume.name, parent, err.to_string()))?;
            fs::write(&path, &file.contents)
                .map_err(|err| materialized_io_error(&volume.name, &path, err.to_string()))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(file.mode))
                .map_err(|err| materialized_io_error(&volume.name, &path, err.to_string()))?;
        }

        Ok(volume_dir.display().to_string())
    }

    pub(crate) fn cleanup_materialized_volumes(&self, ship_id: &str) -> Result<(), ReconcileError> {
        let root = self.materialized_ship_dir(ship_id);
        match fs::remove_dir_all(&root) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(materialized_io_error("<all>", &root, err.to_string())),
        }
    }

    fn materialized_ship_dir(&self, ship_id: &str) -> PathBuf {
        self.volume_data_dir.join(ship_id).join(".materialized")
    }

    fn materialized_volume_dir(&self, ship_id: &str, volume_name: &str) -> PathBuf {
        self.materialized_ship_dir(ship_id).join(volume_name)
    }
}

fn create_dir_with_mode(path: &Path, mode: u32) -> io::Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
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
    use super::create_dir_with_mode;
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
}
