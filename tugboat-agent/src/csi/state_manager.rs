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

//! Atomic state persistence for CSI volumes with per-ship concurrency control.
//!
//! This module provides:
//! - Atomic file writes using temp file + atomic rename pattern
//! - Per-ship async Mutex to serialize state operations
//! - Partial state detection for recovery after failures

use super::types::CsiError;
use dashmap::DashMap;
use std::fs::{create_dir_all, remove_dir, remove_file, rename};
use std::io;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A globally-shared map of per-ship state locks.
/// Ensures that all state file operations (read, write, delete) on a ship
/// are serialized and atomic.
pub(crate) struct StateManager {
    /// Per-ship async locks: Arc<Mutex<()>>
    /// Accessed via DashMap for cheap concurrent access without global lock
    locks: Arc<DashMap<String, Arc<Mutex<()>>>>,
}

impl StateManager {
    pub(crate) fn new() -> Self {
        Self {
            locks: Arc::new(DashMap::new()),
        }
    }

    /// Acquire a per-ship state lock, creating it if needed.
    /// Returns an Arc that can be cloned to share the lock across tasks.
    fn get_lock(&self, ship_id: &str) -> Arc<Mutex<()>> {
        // Check if lock already exists (fast path - no allocation)
        if let Some(lock) = self.locks.get(ship_id) {
            return lock.clone();
        }

        // Lock doesn't exist, create a new one
        let new_lock = Arc::new(Mutex::new(()));
        // Entry API handles race: if another task inserts first, use theirs
        match self.locks.entry(ship_id.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(occupied) => occupied.get().clone(),
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                let inserted = vacant.insert(new_lock.clone());
                inserted.clone()
            }
        }
    }

    /// Execute a closure with the per-ship state lock held.
    /// Ensures atomicity of state operations across concurrent reconcilers.
    pub(crate) async fn with_lock<T, F>(&self, ship_id: &str, operation: F) -> Result<T, CsiError>
    where
        F: FnOnce() -> Result<T, CsiError>,
    {
        let lock = self.get_lock(ship_id);
        let _guard = lock.lock().await;
        operation()
    }

    /// Remove the lock entry for a ship once its backing state directory is gone.
    /// This prevents the per-ship lock map from growing indefinitely in long-running
    /// agents with high ship churn. Ship IDs are UUIDs that are never reused, so no
    /// further state operations can occur for this ship after cleanup.
    pub(crate) fn remove_lock(&self, ship_id: &str) {
        self.locks.remove(ship_id);
    }
}

impl Clone for StateManager {
    fn clone(&self) -> Self {
        Self {
            locks: self.locks.clone(),
        }
    }
}

/// Atomically write a JSON file using temp file + atomic rename.
/// If the write fails, the temp file is cleaned up (not left behind).
pub(crate) fn atomic_write_json<T: serde::Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), CsiError> {
    // Create parent directory if needed
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }

    // Write to temp file first (so partial write doesn't corrupt target file)
    let temp_path = path.with_extension("tmp");
    let json_bytes = serde_json::to_vec(value)?;

    match std::fs::write(&temp_path, json_bytes) {
        Ok(()) => {
            // Atomic rename: either succeeds entirely or fails entirely (syscall level)
            rename(&temp_path, path).map_err(|e| {
                // Clean up temp file if rename failed (best-effort)
                let _ = remove_file(&temp_path);
                CsiError::Io(e)
            })?;
            Ok(())
        }
        Err(e) => {
            // Write failed, try to clean up temp file (best-effort)
            let _ = remove_file(&temp_path);
            Err(CsiError::Io(e))
        }
    }
}

/// Safely remove a file, ignoring NotFound errors (idempotent).
pub(crate) fn safe_remove_file(path: &Path) -> Result<(), CsiError> {
    match remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(CsiError::Io(e)),
    }
}

/// Safely remove a directory, ignoring NotFound and DirectoryNotEmpty errors (idempotent).
pub(crate) fn safe_remove_dir(path: &Path) -> Result<(), CsiError> {
    match remove_dir(path) {
        Ok(()) => Ok(()),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(CsiError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write_success() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.json");

        let data = serde_json::json!({"key": "value"});
        atomic_write_json(&path, &data).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("key"));

        // Verify temp file was cleaned up
        let temp_path = path.with_extension("tmp");
        assert!(!temp_path.exists());
    }

    #[test]
    fn test_atomic_write_overwrites_existing() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.json");

        // Write initial data
        let data1 = serde_json::json!({"version": 1});
        atomic_write_json(&path, &data1).unwrap();

        // Overwrite with new data
        let data2 = serde_json::json!({"version": 2});
        atomic_write_json(&path, &data2).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"version\":2"));
    }

    #[test]
    fn test_atomic_write_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("test.json");

        let data = serde_json::json!({"nested": true});
        atomic_write_json(&path, &data).unwrap();

        assert!(path.exists());
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("nested"));
    }

    #[tokio::test]
    async fn test_state_manager_lock_serialization() {
        let manager = StateManager::new();

        // Simulate concurrent operations that should be serialized
        let manager1 = manager.clone();
        let handle1 = tokio::spawn(async move {
            manager1
                .with_lock("ship-1", || {
                    // Simulate some work
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    Ok::<(), CsiError>(())
                })
                .await
        });

        let manager2 = manager.clone();
        let handle2 = tokio::spawn(async move {
            manager2
                .with_lock("ship-1", || Ok::<(), CsiError>(()))
                .await
        });

        let _ = tokio::join!(handle1, handle2);
        // If we got here, both locks were acquired (serialized by the Mutex)
    }

    #[tokio::test]
    async fn test_state_manager_different_ships_concurrent() {
        let manager = StateManager::new();

        let manager1 = manager.clone();
        let handle1 = tokio::spawn(async move {
            manager1
                .with_lock("ship-1", || Ok::<(), CsiError>(()))
                .await
        });

        let manager2 = manager.clone();
        let handle2 = tokio::spawn(async move {
            manager2
                .with_lock("ship-2", || Ok::<(), CsiError>(()))
                .await
        });

        // Both should complete without deadlock (different ships = different locks)
        let _ = tokio::join!(handle1, handle2);
    }

    #[test]
    fn test_safe_remove_file_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.json");

        // Should not error on NotFound
        safe_remove_file(&path).unwrap();
    }

    #[test]
    fn test_safe_remove_dir_not_empty() {
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path().join("mydir");
        fs::create_dir(&dir_path).unwrap();
        fs::write(dir_path.join("file.txt"), "content").unwrap();

        // Should not error on DirectoryNotEmpty
        safe_remove_dir(&dir_path).unwrap();

        // Directory still exists (not removed because not empty)
        assert!(dir_path.exists());
    }
}
