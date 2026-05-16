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

pub mod v1 {
    use crate::validators::{NameValidator, NamespaceProhibitedValidator, Validator};
    use crate::{apply_resource, apply_validators, resource_api};

    include!(concat!(env!("OUT_DIR"), "/tugboat.snapshot.v1.rs"));

    apply_resource!(VolumeSnapshot, resource_api::VOLUME_SNAPSHOT, namespaced);
    apply_resource!(
        VolumeSnapshotContent,
        resource_api::VOLUME_SNAPSHOT_CONTENT,
        cluster
    );
    apply_resource!(
        VolumeSnapshotClass,
        resource_api::VOLUME_SNAPSHOT_CLASS,
        cluster
    );

    apply_validators!(
        VolumeSnapshot,
        validators NameValidator,
        VolumeSnapshotSourceValidator
    );
    apply_validators!(
        VolumeSnapshotContent,
        validators NameValidator,
        NamespaceProhibitedValidator,
        VolumeSnapshotContentSpecValidator
    );
    apply_validators!(
        VolumeSnapshotClass,
        validators NameValidator,
        NamespaceProhibitedValidator,
        VolumeSnapshotClassSpecValidator
    );

    pub struct VolumeSnapshotSourceValidator;
    impl Validator<VolumeSnapshot> for VolumeSnapshotSourceValidator {
        fn validate(&self, value: &VolumeSnapshot) -> bool {
            let Some(source) = value.spec.as_ref().and_then(|spec| spec.source.as_ref()) else {
                return false;
            };
            exactly_one_set(
                source.persistent_volume_claim_name.as_ref(),
                source.volume_snapshot_content_name.as_ref(),
            )
        }
    }

    pub struct VolumeSnapshotContentSpecValidator;
    impl Validator<VolumeSnapshotContent> for VolumeSnapshotContentSpecValidator {
        fn validate(&self, value: &VolumeSnapshotContent) -> bool {
            let Some(spec) = value.spec.as_ref() else {
                return false;
            };
            if !valid_deletion_policy(&spec.deletion_policy) {
                return false;
            }
            let Some(source) = spec.source.as_ref() else {
                return false;
            };
            exactly_one_set(
                source.volume_handle.as_ref(),
                source.snapshot_handle.as_ref(),
            )
        }
    }

    pub struct VolumeSnapshotClassSpecValidator;
    impl Validator<VolumeSnapshotClass> for VolumeSnapshotClassSpecValidator {
        fn validate(&self, value: &VolumeSnapshotClass) -> bool {
            value
                .spec
                .as_ref()
                .is_some_and(|spec| valid_deletion_policy(&spec.deletion_policy))
        }
    }

    fn exactly_one_set(left: Option<&String>, right: Option<&String>) -> bool {
        left.is_some() ^ right.is_some()
    }

    fn valid_deletion_policy(value: &str) -> bool {
        matches!(value, "Retain" | "Delete")
    }

    #[cfg(test)]
    mod tests {
        use super::{
            VolumeSnapshot, VolumeSnapshotClass, VolumeSnapshotClassSpec, VolumeSnapshotContent,
            VolumeSnapshotContentSource, VolumeSnapshotContentSpec, VolumeSnapshotSource,
        };
        use crate::manifests::meta::v1::ObjectMeta;
        use crate::validators::Validatable;

        #[test]
        fn volume_snapshot_requires_exactly_one_source() {
            for (pvc, content, valid) in [
                (Some("claim-a"), None, true),
                (None, Some("content-a"), true),
                (None, None, false),
                (Some("claim-a"), Some("content-a"), false),
            ] {
                let snapshot = VolumeSnapshot {
                    object_meta: Some(ObjectMeta {
                        name: Some("snap-a".to_string()),
                        namespace: Some("default".to_string()),
                        ..Default::default()
                    }),
                    spec: Some(super::VolumeSnapshotSpec {
                        source: Some(VolumeSnapshotSource {
                            persistent_volume_claim_name: pvc.map(str::to_string),
                            volume_snapshot_content_name: content.map(str::to_string),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                };

                assert_eq!(
                    snapshot.validate(),
                    valid,
                    "pvc={pvc:?} content={content:?}"
                );
            }
        }

        #[test]
        fn volume_snapshot_content_validates_source_policy_and_namespace() {
            let content = VolumeSnapshotContent {
                object_meta: Some(ObjectMeta {
                    name: Some("content-a".to_string()),
                    ..Default::default()
                }),
                spec: Some(VolumeSnapshotContentSpec {
                    deletion_policy: "Delete".to_string(),
                    source: Some(VolumeSnapshotContentSource {
                        volume_handle: Some("vol-a".to_string()),
                        snapshot_handle: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert!(content.validate());

            let mut both_sources = content.clone();
            both_sources.spec.as_mut().unwrap().source = Some(VolumeSnapshotContentSource {
                volume_handle: Some("vol-a".to_string()),
                snapshot_handle: Some("snap-a".to_string()),
            });
            assert!(!both_sources.validate());

            let mut invalid_policy = content.clone();
            invalid_policy.spec.as_mut().unwrap().deletion_policy = "Archive".to_string();
            assert!(!invalid_policy.validate());

            let mut namespaced = content;
            namespaced.object_meta.as_mut().unwrap().namespace = Some("default".to_string());
            assert!(!namespaced.validate());
        }

        #[test]
        fn volume_snapshot_class_validates_policy_and_namespace() {
            for policy in ["Retain", "Delete"] {
                let class = VolumeSnapshotClass {
                    object_meta: Some(ObjectMeta {
                        name: Some("snapclass".to_string()),
                        ..Default::default()
                    }),
                    spec: Some(VolumeSnapshotClassSpec {
                        deletion_policy: policy.to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                assert!(class.validate(), "policy={policy}");
            }

            let invalid = VolumeSnapshotClass {
                object_meta: Some(ObjectMeta {
                    name: Some("snapclass".to_string()),
                    ..Default::default()
                }),
                spec: Some(VolumeSnapshotClassSpec {
                    deletion_policy: "Archive".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            assert!(!invalid.validate());
        }
    }
}
