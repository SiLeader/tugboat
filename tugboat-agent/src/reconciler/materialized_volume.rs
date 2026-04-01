use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::volume::MaterializedVolumeInfo;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

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

    fn cleanup_stale_files(
        &self,
        dir: &Path,
        desired_paths: &HashSet<PathBuf>,
    ) -> io::Result<()> {
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

fn write_file_atomically(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::Other, "file has no parent directory")
    })?;
    let tmp_path = parent.join(format!(
        ".{}.tugboat-tmp",
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
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

fn materialized_io_error(volume: &str, path: &Path, reason: String) -> ReconcileError {
    ReconcileError::MaterializedVolumeIo {
        volume: volume.to_string(),
        path: path.display().to_string(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::{create_dir_with_mode, write_file_atomically};
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
}
