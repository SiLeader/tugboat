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

use crate::validate::validate_safe_id;
use std::path::{Path, PathBuf};

pub const DEFAULT_SNAPSHOT_DIR: &str = "/var/lib/tugboat/snapshots";

/// Returns the per-Ship staging directory under `snapshot_dir`.
pub fn ship_snapshot_dir(snapshot_dir: &Path, ship_id: &str) -> Result<PathBuf, crate::Error> {
    validate_safe_id(ship_id, "ship_id")?;
    Ok(snapshot_dir.join(ship_id))
}

/// Returns the path that should hold a snapshot artifact for the given
/// `ship_id` / `handle` pair. Both inputs are validated to prevent path
/// traversal.
pub fn snapshot_artifact_path(
    snapshot_dir: &Path,
    ship_id: &str,
    handle: &str,
) -> Result<PathBuf, crate::Error> {
    let ship_dir = ship_snapshot_dir(snapshot_dir, ship_id)?;
    validate_handle(handle)?;
    Ok(ship_dir.join(handle))
}

fn validate_handle(handle: &str) -> Result<(), crate::Error> {
    if handle.is_empty() {
        return Err(crate::Error::Validation(
            "handle must not be empty".to_string(),
        ));
    }
    if handle.starts_with('.') {
        return Err(crate::Error::Validation(
            "handle must not start with '.'".to_string(),
        ));
    }
    if handle.contains("..") {
        return Err(crate::Error::Validation(
            "handle must not contain '..'".to_string(),
        ));
    }
    if handle.contains('/') {
        return Err(crate::Error::Validation(
            "handle must not contain '/'".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_paths_join_under_ship_dir() {
        let base = PathBuf::from("/var/lib/tugboat/snapshots");
        let path = snapshot_artifact_path(&base, "ship-uid", "snap-1").unwrap();
        assert_eq!(
            path,
            PathBuf::from("/var/lib/tugboat/snapshots/ship-uid/snap-1")
        );
    }

    #[test]
    fn snapshot_paths_reject_path_traversal() {
        let base = PathBuf::from("/var/lib/tugboat/snapshots");
        assert!(snapshot_artifact_path(&base, "../etc", "snap").is_err());
        assert!(snapshot_artifact_path(&base, "ship", "../etc").is_err());
        assert!(snapshot_artifact_path(&base, "ship", "a/b").is_err());
        assert!(snapshot_artifact_path(&base, "ship", "").is_err());
    }
}
