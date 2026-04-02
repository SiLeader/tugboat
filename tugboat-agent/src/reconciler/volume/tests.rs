use super::normalize::{
    MaterializedFile, MaterializedVolumeSourceKind, build_materialized_files,
    normalized_ship_volumes, validate_materialized_volume_name, validate_relative_target_path,
};
use super::{
    decode_secret_volume_data, effective_volume_mode, ensure_access_modes_compatible,
    ensure_supported_claim_mode, ensure_supported_csi_source,
    ensure_supported_persistent_volume_mode, ensure_volume_claim_binding,
};
use tugboat_resources::manifests::core::v1::{
    ConfigMapVolumeSource, CsiPersistentVolumeSource, KeyToPath, PersistentVolumeClaimReference,
    PersistentVolumeClaimVolumeSource, Secret, SecretReference, SecretVolumeSource, ShipSpec,
    ShipVolume, ShipVolumeClaimReference,
};

#[test]
fn defaults_volume_mode_to_block() {
    assert_eq!(effective_volume_mode(None), "Block");
}

#[test]
fn allows_supported_volume_modes() {
    assert!(ensure_supported_claim_mode("claim", "Filesystem").is_ok());
    assert!(ensure_supported_persistent_volume_mode("volume", "Filesystem").is_ok());
}

#[test]
fn requires_volume_to_be_bound_to_exact_claim() {
    let claim_ref = PersistentVolumeClaimReference {
        name: "data".to_string(),
        namespace: "alpha".to_string(),
    };

    assert!(ensure_volume_claim_binding("pv-1", Some(&claim_ref), "alpha", "data").is_ok());
    assert!(ensure_volume_claim_binding("pv-1", Some(&claim_ref), "beta", "data").is_err());
    assert!(ensure_volume_claim_binding("pv-1", None, "alpha", "data").is_err());
}

#[test]
fn rejects_access_mode_mismatches() {
    assert!(
        ensure_access_modes_compatible(
            "claim",
            &["ReadWriteMany".to_string()],
            "pv",
            &["ReadOnlyMany".to_string()],
        )
        .is_err()
    );
}

#[test]
fn allows_controller_expand_secret_ref() {
    let source = CsiPersistentVolumeSource {
        controller_expand_secret_ref: Some(SecretReference {
            name: "expand-secret".to_string(),
            namespace: "alpha".to_string(),
        }),
        ..Default::default()
    };

    assert!(ensure_supported_csi_source("pv", &source).is_ok());
}

#[test]
fn allows_node_publish_and_stage_secrets() {
    let source = CsiPersistentVolumeSource {
        node_publish_secret_ref: Some(SecretReference {
            name: "publish-secret".to_string(),
            namespace: "alpha".to_string(),
        }),
        node_stage_secret_ref: Some(SecretReference {
            name: "stage-secret".to_string(),
            namespace: "alpha".to_string(),
        }),
        fs_type: Some("xfs".to_string()),
        volume_attributes: std::collections::HashMap::from([(
            "storage.kubernetes.io/csiProvisionerIdentity".to_string(),
            "test".to_string(),
        )]),
        ..Default::default()
    };

    assert!(ensure_supported_csi_source("pv", &source).is_ok());
}

#[test]
fn decodes_binary_secret_volume_data() {
    let secret = Secret {
        data: std::collections::HashMap::from([("cert".to_string(), "AAE=".to_string())]),
        string_data: std::collections::HashMap::from([("token".to_string(), "plain".to_string())]),
        ..Default::default()
    };

    let decoded = decode_secret_volume_data("secret-vol", "alpha", "app-secret", secret)
        .expect("secret decoding should succeed");
    assert_eq!(decoded.get("cert"), Some(&vec![0, 1]));
    assert_eq!(decoded.get("token"), Some(&b"plain".to_vec()));
}

#[test]
fn normalizes_legacy_and_named_volumes() {
    let spec = ShipSpec {
        volume_claim_ref: vec![ShipVolumeClaimReference {
            name: "legacy-data".to_string(),
        }],
        volumes: vec![
            ShipVolume {
                name: "cfg".to_string(),
                config_map: Some(ConfigMapVolumeSource {
                    name: "app-config".to_string(),
                    items: vec![KeyToPath {
                        key: "app.toml".to_string(),
                        path: "config/app.toml".to_string(),
                    }],
                    default_mode: Some(0o640),
                    optional: Some(true),
                }),
                ..Default::default()
            },
            ShipVolume {
                name: "secret".to_string(),
                secret: Some(SecretVolumeSource {
                    secret_name: "app-secret".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ShipVolume {
                name: "pvc".to_string(),
                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                    claim_name: "data".to_string(),
                }),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let normalized = normalized_ship_volumes(&spec).expect("volume normalization must succeed");
    assert_eq!(normalized.len(), 4);
    assert_eq!(normalized[0].name, "legacy-data");
    assert_eq!(normalized[1].name, "cfg");
    assert_eq!(normalized[2].name, "secret");
    assert_eq!(normalized[3].name, "pvc");
}

#[test]
fn detects_config_map_materialized_volume_references() {
    let spec = ShipSpec {
        volumes: vec![
            ShipVolume {
                name: "cfg".to_string(),
                config_map: Some(ConfigMapVolumeSource {
                    name: "app-config".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ShipVolume {
                name: "secret".to_string(),
                secret: Some(SecretVolumeSource {
                    secret_name: "app-secret".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    assert!(
        super::ship_references_materialized_resource(
            &spec,
            MaterializedVolumeSourceKind::ConfigMap,
            "app-config",
        )
        .expect("config map lookup should succeed")
    );
    assert!(
        !super::ship_references_materialized_resource(
            &spec,
            MaterializedVolumeSourceKind::ConfigMap,
            "other-config",
        )
        .expect("config map lookup should succeed")
    );
}

#[test]
fn detects_secret_materialized_volume_references() {
    let spec = ShipSpec {
        volumes: vec![
            ShipVolume {
                name: "cfg".to_string(),
                config_map: Some(ConfigMapVolumeSource {
                    name: "app-config".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ShipVolume {
                name: "secret".to_string(),
                secret: Some(SecretVolumeSource {
                    secret_name: "app-secret".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    assert!(
        super::ship_references_materialized_resource(
            &spec,
            MaterializedVolumeSourceKind::Secret,
            "app-secret",
        )
        .expect("secret lookup should succeed")
    );
    assert!(
        !super::ship_references_materialized_resource(
            &spec,
            MaterializedVolumeSourceKind::Secret,
            "other-secret",
        )
        .expect("secret lookup should succeed")
    );
}

#[test]
fn ignores_persistent_volume_claims_when_matching_materialized_references() {
    let spec = ShipSpec {
        volume_claim_ref: vec![ShipVolumeClaimReference {
            name: "data".to_string(),
        }],
        volumes: vec![ShipVolume {
            name: "pvc".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "data".to_string(),
            }),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(
        !super::ship_references_materialized_resource(
            &spec,
            MaterializedVolumeSourceKind::ConfigMap,
            "data",
        )
        .expect("config map lookup should succeed")
    );
    assert!(
        !super::ship_references_materialized_resource(
            &spec,
            MaterializedVolumeSourceKind::Secret,
            "data",
        )
        .expect("secret lookup should succeed")
    );
}

#[test]
fn rejects_multiple_named_volume_sources() {
    let spec = ShipSpec {
        volumes: vec![ShipVolume {
            name: "invalid".to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: "cfg".to_string(),
                ..Default::default()
            }),
            secret: Some(SecretVolumeSource {
                secret_name: "secret".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(normalized_ship_volumes(&spec).is_err());
}

#[test]
fn rejects_duplicate_named_volume_paths() {
    let err = normalized_ship_volumes(&ShipSpec {
        volumes: vec![ShipVolume {
            name: "cfg".to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: "app-config".to_string(),
                items: vec![
                    KeyToPath {
                        key: "a".to_string(),
                        path: "config/app".to_string(),
                    },
                    KeyToPath {
                        key: "b".to_string(),
                        path: "config/app".to_string(),
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap_err();

    assert!(format!("{err}").contains("duplicate target path"));
}

#[test]
fn rejects_parent_dir_volume_paths() {
    assert!(validate_relative_target_path("cfg", "../etc/passwd").is_err());
    assert!(validate_relative_target_path("cfg", "/etc/passwd").is_err());
    assert!(validate_relative_target_path("cfg", "a/./b").is_err());
}

#[test]
fn rejects_unsafe_volume_names() {
    assert!(validate_materialized_volume_name("../cfg").is_err());
    assert!(validate_materialized_volume_name("/etc/passwd").is_err());
    assert!(validate_materialized_volume_name("nested/cfg").is_err());
    assert!(validate_materialized_volume_name("cfg/").is_err());
    assert!(validate_materialized_volume_name("cfg").is_ok());
    assert!(validate_materialized_volume_name("cfg.v1").is_ok());
}

#[test]
fn rejects_named_volumes_with_unsafe_names() {
    let err = normalized_ship_volumes(&ShipSpec {
        volumes: vec![ShipVolume {
            name: "../cfg".to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: "app-config".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap_err();

    assert!(format!("{err}").contains("single relative path segment"));
}

#[test]
fn materializes_all_keys_when_items_are_empty() {
    let mut files = build_materialized_files(
        "cfg",
        MaterializedVolumeSourceKind::ConfigMap,
        "app-config",
        std::collections::HashMap::from([
            ("app.toml".to_string(), b"[app]".to_vec()),
            ("log.toml".to_string(), b"[log]".to_vec()),
        ]),
        &[],
        0o640,
        false,
    )
    .expect("file projection should succeed");
    files.sort_by(|left, right| left.path.cmp(&right.path));

    assert_eq!(
        files,
        vec![
            MaterializedFile {
                path: "app.toml".to_string(),
                contents: b"[app]".to_vec(),
                mode: 0o640,
            },
            MaterializedFile {
                path: "log.toml".to_string(),
                contents: b"[log]".to_vec(),
                mode: 0o640,
            },
        ]
    );
}

#[test]
fn projects_selected_keys_to_custom_paths() {
    let files = build_materialized_files(
        "cfg",
        MaterializedVolumeSourceKind::ConfigMap,
        "app-config",
        std::collections::HashMap::from([("app".to_string(), b"hello".to_vec())]),
        &[super::NormalizedKeyToPath {
            key: "app".to_string(),
            path: "config/app.txt".to_string(),
        }],
        0o600,
        false,
    )
    .expect("file projection should succeed");

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "config/app.txt");
    assert_eq!(files[0].contents, b"hello".to_vec());
    assert_eq!(files[0].mode, 0o600);
}
