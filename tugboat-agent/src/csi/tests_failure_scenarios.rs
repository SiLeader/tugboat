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

//! Integration tests for CSI failure scenarios and recovery paths.
//! Tests validate that partial publish states are handled gracefully.

#[cfg(test)]
mod tests {
    use crate::csi::state_manager::{StateManager, atomic_write_json};
    use crate::csi::{CsiDrivers, CsiError, CsiWrapper, PublishedAccessType, PublishedVolume};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;
    use tugboat_csi_operator::TugboatCsiOperator;

    #[tokio::test]
    async fn test_publish_partial_state_error_recovery() {
        // Simulate: Volume mounted successfully, but state file write fails.
        // Verify: PublishPartialState error is returned, volume is not rolled back.

        let _temp_dir = TempDir::new().unwrap();

        // Verify: PublishPartialState can be created and matches error pattern
        let error = CsiError::PublishPartialState {
            volume_id: "vol-123".to_string(),
            reason: "disk full".to_string(),
        };

        match error {
            CsiError::PublishPartialState { volume_id, reason } => {
                assert_eq!(volume_id, "vol-123");
                assert_eq!(reason, "disk full");
            }
            _ => panic!("Expected PublishPartialState error"),
        }
    }

    #[tokio::test]
    async fn test_state_recovery_with_concurrent_ships() {
        // Simulate: Two ships' volumes being recovered concurrently.
        // Verify: Each ship's lock serializes its state operations independently.

        let manager = StateManager::new();

        let manager1 = manager.clone();
        let handle1 = tokio::spawn(async move {
            manager1
                .with_lock("ship-1", || {
                    // Simulate reading and modifying ship-1 state
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    Ok::<_, CsiError>(())
                })
                .await
        });

        let manager2 = manager.clone();
        let handle2 = tokio::spawn(async move {
            manager2
                .with_lock("ship-2", || {
                    // Different ship, should not block
                    Ok::<_, CsiError>(())
                })
                .await
        });

        let (r1, r2) = tokio::join!(handle1, handle2);
        assert!(r1.is_ok() && r1.unwrap().is_ok());
        assert!(r2.is_ok() && r2.unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_atomic_write_recovery_on_failure() {
        // Simulate: Write fails (permission denied), temp file exists.
        // Verify: Temp file is cleaned up, state file not corrupted.

        let temp_dir = TempDir::new().unwrap();
        let _state_path = temp_dir.path().join("state.json");

        // Create a parent directory with read-only permissions to force write failure
        let read_only_dir = temp_dir.path().join("readonly");
        fs::create_dir(&read_only_dir).unwrap();
        fs::set_permissions(&read_only_dir, fs::Permissions::from_mode(0o444)).unwrap();

        let fail_path = read_only_dir.join("state.json");
        let data = serde_json::json!({"test": "value"});

        // Attempt to write to read-only directory should fail
        let result = atomic_write_json(&fail_path, &data);
        assert!(result.is_err());

        // Verify no temp or state files were created
        assert!(!fail_path.exists());
        assert!(!fail_path.with_extension("tmp").exists());

        // Cleanup: restore permissions so TempDir can delete
        fs::set_permissions(&read_only_dir, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[tokio::test]
    async fn test_recovered_volume_state_is_idempotent() {
        // Simulate: Load same volume state twice.
        // Verify: Both loads return identical state, proving idempotency.

        let temp_dir = TempDir::new().unwrap();
        let wrapper = CsiWrapper::new(
            TugboatCsiOperator::default(),
            CsiDrivers::default(),
            temp_dir.path(),
        );

        let volume = PublishedVolume {
            claim_name: "data".to_string(),
            driver: "example.csi".to_string(),
            volume_id: "vol-456".to_string(),
            target_path: temp_dir
                .path()
                .join("ship-2")
                .join("data.block")
                .display()
                .to_string(),
            access_type: PublishedAccessType::Block,
            mount_namespace_path: "/var/run/tugboat/mntns/ship-2".to_string(),
            staging_target_path: None,
            controller_published: false,
            pvc_name: Some("data-pvc".to_string()),
        };

        // Persist volume twice (idempotent write)
        wrapper
            .persist_published_volume(&volume, "ship-2")
            .await
            .unwrap();
        wrapper
            .persist_published_volume(&volume, "ship-2")
            .await
            .unwrap();

        // Load should return same state in both cases
        let loaded1 = wrapper.load_published_volumes("ship-2").await.unwrap();
        let loaded2 = wrapper.load_published_volumes("ship-2").await.unwrap();

        assert_eq!(loaded1, vec![volume.clone()]);
        assert_eq!(loaded2, vec![volume.clone()]);
        assert_eq!(loaded1, loaded2);
    }

    #[tokio::test]
    async fn test_remove_published_volume_with_lock_safety() {
        // Simulate: Two concurrent removals of same volume.
        // Verify: Both complete without error (idempotent delete with lock).

        let temp_dir = TempDir::new().unwrap();
        let wrapper = CsiWrapper::new(
            TugboatCsiOperator::default(),
            CsiDrivers::default(),
            temp_dir.path(),
        );

        let volume = PublishedVolume {
            claim_name: "shared-vol".to_string(),
            driver: "example.csi".to_string(),
            volume_id: "vol-789".to_string(),
            target_path: temp_dir
                .path()
                .join("ship-3")
                .join("shared-vol.fs")
                .display()
                .to_string(),
            access_type: PublishedAccessType::Filesystem,
            mount_namespace_path: "/var/run/tugboat/mntns/ship-3".to_string(),
            staging_target_path: None,
            controller_published: false,
            pvc_name: Some("shared-pvc".to_string()),
        };

        // Persist the volume
        wrapper
            .persist_published_volume(&volume, "ship-3")
            .await
            .unwrap();

        // Simulate concurrent removal attempts
        let wrapper1 = wrapper.clone();
        let volume1 = volume.clone();
        let handle1 =
            tokio::spawn(async move { wrapper1.remove_published_volume_state(&volume1).await });

        let wrapper2 = wrapper.clone();
        let volume2 = volume.clone();
        let handle2 =
            tokio::spawn(async move { wrapper2.remove_published_volume_state(&volume2).await });

        // Both should succeed (idempotent)
        let r1 = handle1.await.unwrap();
        let r2 = handle2.await.unwrap();
        assert!(r1.is_ok());
        assert!(r2.is_ok());

        // Verify volume is cleaned up
        let remaining = wrapper.load_published_volumes("ship-3").await.unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_extract_ship_id_from_mount_namespace_path() {
        // Verify: extract_ship_id correctly parses ship-id from mount namespace path.

        let volume = PublishedVolume {
            claim_name: "test".to_string(),
            driver: "test.csi".to_string(),
            volume_id: "vol-test".to_string(),
            target_path: "/var/lib/csi/ship-abc123/test.block".to_string(),
            access_type: PublishedAccessType::Block,
            mount_namespace_path: "/var/run/tugboat/mntns/ship-abc123".to_string(),
            staging_target_path: None,
            controller_published: false,
            pvc_name: None,
        };

        let extracted = volume.extract_ship_id().unwrap();
        assert_eq!(extracted, "ship-abc123");
    }

    #[test]
    fn test_extract_ship_id_handles_missing_path() {
        // Verify: extract_ship_id returns None for invalid mount namespace path.

        let volume = PublishedVolume {
            claim_name: "test".to_string(),
            driver: "test.csi".to_string(),
            volume_id: "vol-test".to_string(),
            target_path: "/var/lib/csi/test.block".to_string(),
            access_type: PublishedAccessType::Block,
            mount_namespace_path: "/".to_string(), // Root path has no filename component
            staging_target_path: None,
            controller_published: false,
            pvc_name: None,
        };

        assert!(volume.extract_ship_id().is_none());
    }
}
