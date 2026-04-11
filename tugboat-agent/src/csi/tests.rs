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

use super::{
    CsiDrivers, CsiWrapper, PublishedAccessType, PublishedVolume, TryConvertFromString,
    access_type_from_volume_mode, cleanup_directory_path, cleanup_target_path,
    effective_publish_settings, filesystem_type, is_retryable_driver_error, prepare_directory_path,
    prepare_target_path, select_access_mode,
};
use crate::csi::CsiError;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tugboat_csi_operator::{CsiAccessMode, CsiAccessType, TugboatCsiOperator};
use tugboat_resources::manifests::core::v1::CsiPersistentVolumeSource;

#[test]
fn can_convert_access_modes() {
    assert!(matches!(
        CsiAccessMode::try_convert_from_string("ReadWriteOnce"),
        Ok(CsiAccessMode::ReadWriteOnce)
    ));
    assert!(matches!(
        CsiAccessType::try_convert_from_string("Block"),
        Ok(CsiAccessType::Block)
    ));
    assert!(matches!(
        access_type_from_volume_mode(Some("Filesystem")),
        Ok(CsiAccessType::Filesystem)
    ));
}

#[test]
fn can_plan_publish_target_path() {
    let wrapper = CsiWrapper::new(
        TugboatCsiOperator::default(),
        CsiDrivers::default(),
        "/var/lib/tugboat-agent/csi",
    );
    let published = wrapper
        .plan_published_volume(
            "ship-uid",
            "data-volume",
            "my-pvc",
            &CsiPersistentVolumeSource {
                driver: "example.csi".to_string(),
                volume_handle: "volume-001".to_string(),
                ..Default::default()
            },
            PublishedAccessType::Block,
            false,
        )
        .expect("volume planning should succeed");

    assert_eq!(published.driver, "example.csi");
    assert_eq!(published.volume_id, "volume-001");
    assert_eq!(
        published.target_path,
        "/var/lib/tugboat-agent/csi/ship-uid/data-volume.block"
    );
    assert_eq!(
        published.mount_namespace_path,
        "/var/run/tugboat/mntns/ship-uid"
    );
    assert_eq!(published.access_type, PublishedAccessType::Block);
    assert_eq!(published.staging_target_path, None);
    assert_eq!(published.pvc_name, Some("my-pvc".to_string()));
    assert_eq!(published.effective_pvc_name(), "my-pvc");
}

#[test]
fn can_plan_filesystem_publish_target_path() {
    let wrapper = CsiWrapper::new(
        TugboatCsiOperator::default(),
        CsiDrivers::default(),
        "/var/lib/tugboat-agent/csi",
    );
    let published = wrapper
        .plan_published_volume(
            "ship-uid",
            "data-volume",
            "my-pvc",
            &CsiPersistentVolumeSource {
                driver: "example.csi".to_string(),
                volume_handle: "volume-001".to_string(),
                ..Default::default()
            },
            PublishedAccessType::Filesystem,
            true,
        )
        .expect("volume planning should succeed");

    assert_eq!(
        published.target_path,
        "/var/lib/tugboat-agent/csi/ship-uid/data-volume.fs"
    );
    assert_eq!(
        published.staging_target_path,
        Some("/var/lib/tugboat-agent/csi/ship-uid/.staging/data-volume".to_string())
    );
}

#[test]
fn chooses_effective_access_mode_without_list_order_dependency() {
    let access_mode = select_access_mode(
        &["ReadOnlyMany".to_string(), "ReadWriteOnce".to_string()],
        &["ReadWriteOnce".to_string(), "ReadOnlyMany".to_string()],
    )
    .expect("access mode selection should succeed");

    assert_eq!(access_mode, CsiAccessMode::ReadWriteOnce);
}

#[test]
fn read_only_claims_force_read_only_publish() {
    let (_, read_only) = effective_publish_settings(
        &["ReadOnlyMany".to_string()],
        &["ReadOnlyMany".to_string()],
        false,
    )
    .expect("publish settings should succeed");

    assert!(read_only);
}

#[tokio::test]
async fn can_persist_and_load_published_volume_state() {
    let temp_dir = std::env::temp_dir().join(format!(
        "tugboat-agent-csi-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    let wrapper = CsiWrapper::new(
        TugboatCsiOperator::default(),
        CsiDrivers::default(),
        &temp_dir,
    );
    let volume = PublishedVolume {
        claim_name: "data-volume".to_string(),
        driver: "example.csi".to_string(),
        volume_id: "volume-001".to_string(),
        target_path: temp_dir
            .join("ship-uid")
            .join("data-volume.block")
            .display()
            .to_string(),
        access_type: PublishedAccessType::Block,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: Some(
            temp_dir
                .join("ship-uid")
                .join(".staging")
                .join("data-volume")
                .display()
                .to_string(),
        ),
        controller_published: false,
        pvc_name: Some("my-pvc".to_string()),
    };

    wrapper
        .persist_published_volume(&volume, "ship-uid")
        .await
        .expect("state persistence should succeed");

    let loaded = wrapper
        .load_published_volumes("ship-uid")
        .await
        .expect("state loading should succeed");

    assert_eq!(loaded, vec![volume.clone()]);

    wrapper
        .remove_published_volume_state(&volume)
        .await
        .expect("state cleanup should succeed");
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn can_prepare_and_cleanup_filesystem_target_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "tugboat-agent-csi-fs-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    let target_path = temp_dir.join("ship-uid").join("data-volume.fs");
    let target_path = target_path.display().to_string();

    prepare_target_path(&target_path, PublishedAccessType::Filesystem)
        .expect("target path preparation should succeed");
    assert!(std::path::Path::new(&target_path).is_dir());

    cleanup_target_path(&target_path, PublishedAccessType::Filesystem)
        .expect("target path cleanup should succeed");
    assert!(!std::path::Path::new(&target_path).exists());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn can_prepare_and_cleanup_block_target_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "tugboat-agent-csi-block-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    let target_path = temp_dir.join("ship-uid").join("data-volume.block");
    let target_path = target_path.display().to_string();

    prepare_target_path(&target_path, PublishedAccessType::Block)
        .expect("target path preparation should succeed");
    assert!(std::path::Path::new(&target_path).is_file());

    cleanup_target_path(&target_path, PublishedAccessType::Block)
        .expect("target path cleanup should succeed");
    assert!(!std::path::Path::new(&target_path).exists());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn integration_happy_path_round_trips_block_volume_state() {
    let temp_dir = std::env::temp_dir().join(format!(
        "tugboat-agent-csi-it-happy-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    let wrapper = CsiWrapper::new(
        TugboatCsiOperator::default(),
        CsiDrivers::default(),
        &temp_dir,
    );
    let volume = PublishedVolume {
        claim_name: "data-volume".to_string(),
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
        controller_published: false,
        pvc_name: Some("my-pvc".to_string()),
    };

    prepare_target_path(&volume.target_path, volume.access_type)
        .expect("block target preparation should succeed");
    wrapper
        .persist_published_volume(&volume, "ship-uid")
        .await
        .expect("state persistence should succeed");

    let loaded = wrapper
        .load_published_volumes("ship-uid")
        .await
        .expect("state loading should succeed");
    assert_eq!(loaded, vec![volume.clone()]);

    wrapper
        .remove_published_volume_state(&volume)
        .await
        .expect("state cleanup should succeed");
    cleanup_target_path(&volume.target_path, volume.access_type)
        .expect("block target cleanup should succeed");
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn integration_cleanup_path_removes_filesystem_state_and_paths() {
    let temp_dir = std::env::temp_dir().join(format!(
        "tugboat-agent-csi-it-cleanup-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
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
        claim_name: "data-volume".to_string(),
        driver: "example.csi".to_string(),
        volume_id: "volume-001".to_string(),
        target_path: target_path.display().to_string(),
        access_type: PublishedAccessType::Filesystem,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: Some(staging_target_path.display().to_string()),
        controller_published: false,
        pvc_name: Some("my-pvc".to_string()),
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
        .persist_published_volume(&volume, "ship-uid")
        .await
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
        .await
        .expect("state cleanup should succeed");

    assert!(
        wrapper
            .load_published_volumes("ship-uid")
            .await
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
    assert!(!temp_dir.join("ship-uid").exists());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn uses_filesystem_type_only_for_filesystem_volumes() {
    let source = CsiPersistentVolumeSource {
        fs_type: Some("xfs".to_string()),
        ..Default::default()
    };

    assert_eq!(
        filesystem_type(&source, CsiAccessType::Filesystem),
        Some("xfs".to_string())
    );
    assert_eq!(filesystem_type(&source, CsiAccessType::Block), None);
}

#[test]
fn mount_flags_are_carried_in_resolved_secrets() {
    let source = CsiPersistentVolumeSource {
        mount_options: vec![
            "noatime".to_string(),
            String::new(),
            "nodiratime".to_string(),
        ],
        ..Default::default()
    };
    let mount_flags: Vec<String> = source
        .mount_options
        .iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect();
    assert_eq!(mount_flags, vec!["noatime", "nodiratime"]);
}

#[test]
fn backward_compatible_deserialization_without_pvc_name() {
    let json = r#"{
        "claim_name": "data-volume",
        "driver": "example.csi",
        "volume_id": "volume-001",
        "target_path": "/var/lib/csi/ship-uid/data-volume.block",
        "access_type": "block",
        "mount_namespace_path": "/var/run/tugboat/mntns/ship-uid",
        "controller_published": false
    }"#;

    let volume: PublishedVolume =
        serde_json::from_str(json).expect("deserialization should succeed");
    assert_eq!(volume.pvc_name, None);
    assert_eq!(volume.effective_pvc_name(), "data-volume");
}

#[test]
fn effective_pvc_name_returns_pvc_name_when_present() {
    let volume = PublishedVolume {
        claim_name: "data-alias".to_string(),
        driver: "example.csi".to_string(),
        volume_id: "volume-001".to_string(),
        target_path: "/var/lib/csi/ship-uid/data-alias.block".to_string(),
        access_type: PublishedAccessType::Block,
        mount_namespace_path: "/var/run/tugboat/mntns/ship-uid".to_string(),
        staging_target_path: None,
        controller_published: false,
        pvc_name: Some("actual-pvc".to_string()),
    };
    assert_eq!(volume.effective_pvc_name(), "actual-pvc");
}

#[tokio::test]
async fn retries_transient_csi_errors_before_succeeding() {
    let wrapper = CsiWrapper::new(
        TugboatCsiOperator::default(),
        CsiDrivers::default(),
        "/var/lib/tugboat-agent/csi",
    );
    let attempts = Arc::new(AtomicUsize::new(0));

    let result = wrapper
        .retry_csi_operation("test operation", "volume-1", {
            let attempts = attempts.clone();
            move || {
                let attempts = attempts.clone();
                async move {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt < 2 {
                        return Err(CsiError::Driver(tugboat_csi_operator::Error::Grpc(
                            tonic::Status::unavailable("temporarily unavailable"),
                        )));
                    }
                    Ok(42_u8)
                }
            }
        })
        .await
        .expect("transient CSI failures should be retried");

    assert_eq!(result, 42);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn does_not_retry_non_retryable_csi_errors() {
    let wrapper = CsiWrapper::new(
        TugboatCsiOperator::default(),
        CsiDrivers::default(),
        "/var/lib/tugboat-agent/csi",
    );
    let attempts = Arc::new(AtomicUsize::new(0));

    let err = wrapper
        .retry_csi_operation("test operation", "volume-1", {
            let attempts = attempts.clone();
            move || {
                let attempts = attempts.clone();
                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err::<u8, CsiError>(CsiError::Driver(
                        tugboat_csi_operator::Error::TargetPathAlreadyExists,
                    ))
                }
            }
        })
        .await
        .expect_err("non-retryable CSI failures should fail immediately");

    assert!(matches!(
        err,
        CsiError::Driver(tugboat_csi_operator::Error::TargetPathAlreadyExists)
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn can_recover_partial_published_volume_state_from_existing_paths() {
    let temp_dir = tempfile::TempDir::new().expect("temp dir should be created");
    let wrapper = CsiWrapper::new(
        TugboatCsiOperator::default(),
        CsiDrivers::default(),
        temp_dir.path(),
    );
    let mount_namespace_path = temp_dir.path().join("mntns").join("ship-uid");
    std::fs::create_dir_all(&mount_namespace_path).expect("mount namespace path should exist");
    let staging_target_path = temp_dir
        .path()
        .join("ship-uid")
        .join(".staging")
        .join("data");
    let volume = PublishedVolume {
        claim_name: "data".to_string(),
        driver: "example.csi".to_string(),
        volume_id: "volume-1".to_string(),
        target_path: temp_dir
            .path()
            .join("ship-uid")
            .join("data.fs")
            .display()
            .to_string(),
        access_type: PublishedAccessType::Filesystem,
        mount_namespace_path: mount_namespace_path.display().to_string(),
        staging_target_path: Some(staging_target_path.display().to_string()),
        controller_published: true,
        pvc_name: Some("data-pvc".to_string()),
    };

    prepare_target_path(&volume.target_path, volume.access_type)
        .expect("publish target should be created");
    prepare_directory_path(
        volume
            .staging_target_path
            .as_deref()
            .expect("staging path should exist"),
    )
    .expect("staging path should be created");

    let recovered = wrapper
        .recover_partial_published_volume_state("ship-uid", std::slice::from_ref(&volume))
        .await
        .expect("recovery should succeed")
        .expect("existing publish paths should be recognized");

    assert_eq!(recovered, vec![volume.clone()]);
    let persisted = wrapper
        .load_published_volumes("ship-uid")
        .await
        .expect("recovered state should be persisted");
    assert_eq!(persisted, vec![volume]);
}

#[test]
fn classifies_retryable_driver_errors() {
    assert!(is_retryable_driver_error(
        &tugboat_csi_operator::Error::RpcTimeout
    ));
    assert!(is_retryable_driver_error(
        &tugboat_csi_operator::Error::Grpc(tonic::Status::unavailable("temporary"),)
    ));
    assert!(!is_retryable_driver_error(
        &tugboat_csi_operator::Error::TargetPathAlreadyExists,
    ));
}
