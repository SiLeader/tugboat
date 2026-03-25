#![allow(dead_code, unused_imports)]

#[path = "../src/mountns.rs"]
mod mountns;

mod csi_state_flow {
    include!("../src/csi.rs");

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be monotonic")
                .as_nanos()
        ))
    }

    #[test]
    fn integration_happy_path_round_trips_block_volume_state() {
        let temp_dir = unique_temp_dir("tugboat-agent-csi-it-happy");
        let wrapper = CsiWrapper::new(
            TugboatCsiOperator::default(),
            CsiDrivers::default(),
            &temp_dir,
        );
        let volume = PublishedVolume {
            driver: "example.csi".to_string(),
            volume_id: "volume-001".to_string(),
            target_path: temp_dir
                .join("ship-uid")
                .join("data-volume.block")
                .display()
                .to_string(),
            access_type: PublishedAccessType::Block,
            mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
            staging_target_path: None,
        };

        prepare_target_path(&volume.target_path, volume.access_type)
            .expect("block target preparation should succeed");
        wrapper
            .persist_published_volume(&volume)
            .expect("state persistence should succeed");

        let loaded = wrapper
            .load_published_volumes("ship-uid")
            .expect("state loading should succeed");

        assert_eq!(loaded, vec![volume.clone()]);

        wrapper
            .remove_published_volume_state(&volume)
            .expect("state cleanup should succeed");
        cleanup_target_path(&volume.target_path, volume.access_type)
            .expect("block target cleanup should succeed");
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn integration_cleanup_path_removes_filesystem_state_and_paths() {
        let temp_dir = unique_temp_dir("tugboat-agent-csi-it-cleanup");
        let wrapper = CsiWrapper::new(
            TugboatCsiOperator::default(),
            CsiDrivers::default(),
            &temp_dir,
        );
        let target_path = temp_dir.join("ship-uid").join("data-volume.fs");
        let staging_target_path = temp_dir
            .join("ship-uid")
            .join(".staging")
            .join("data-volume");
        let volume = PublishedVolume {
            driver: "example.csi".to_string(),
            volume_id: "volume-001".to_string(),
            target_path: target_path.display().to_string(),
            access_type: PublishedAccessType::Filesystem,
            mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
            staging_target_path: Some(staging_target_path.display().to_string()),
        };

        prepare_directory_path(
            volume
                .staging_target_path
                .as_deref()
                .expect("stage path should exist"),
        )
        .expect("staging directory preparation should succeed");
        prepare_target_path(&volume.target_path, volume.access_type)
            .expect("filesystem target preparation should succeed");
        wrapper
            .persist_published_volume(&volume)
            .expect("state persistence should succeed");

        cleanup_target_path(&volume.target_path, volume.access_type)
            .expect("filesystem target cleanup should succeed");
        cleanup_directory_path(
            volume
                .staging_target_path
                .as_deref()
                .expect("stage path should exist"),
        )
        .expect("staging directory cleanup should succeed");
        wrapper
            .remove_published_volume_state(&volume)
            .expect("state cleanup should succeed");

        assert!(
            wrapper
                .load_published_volumes("ship-uid")
                .expect("state loading should succeed")
                .is_empty()
        );
        assert!(!std::path::Path::new(&volume.target_path).exists());
        assert!(
            !std::path::Path::new(
                volume
                    .staging_target_path
                    .as_deref()
                    .expect("stage path should exist")
            )
            .exists()
        );
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
