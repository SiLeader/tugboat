use crate::config::{ControllerManagerConfig, ProvisionerConfig};
use crate::error::ControllerError;
use std::collections::HashMap;
use tugboat_csi_operator::{CsiAccessMode, CsiAccessType};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::{
    CsiPersistentVolumeSource, PersistentVolume, PersistentVolumeClaim,
    PersistentVolumeClaimReference, PersistentVolumeSpec, StorageClass,
};
use tugboat_resources::manifests::meta::v1::ObjectMeta;

pub(crate) const MANAGED_LABEL: &str = "storage.tugboat.io/dynamic-provisioned";
pub(crate) const MANAGED_LABEL_VALUE: &str = "true";
pub(crate) const PROVISIONER_ANNOTATION: &str = "storage.tugboat.io/provisioner";
pub(crate) const CLAIM_NAMESPACE_ANNOTATION: &str = "storage.tugboat.io/claim-namespace";
pub(crate) const CLAIM_NAME_ANNOTATION: &str = "storage.tugboat.io/claim-name";
pub(crate) const PV_FINALIZER: &str = "storage.tugboat.io/csi-provisioner";

const DEFAULT_VOLUME_MODE: &str = "Block";
const READ_ONLY_MANY: &str = "ReadOnlyMany";
const READ_WRITE_ONCE: &str = "ReadWriteOnce";
const READ_WRITE_MANY: &str = "ReadWriteMany";
const RECLAIM_POLICY_DELETE: &str = "Delete";

pub(crate) fn pvc_identity(
    pvc: &PersistentVolumeClaim,
) -> Result<(String, String, Option<String>), ControllerError> {
    let namespace = pvc
        .namespace()
        .ok_or(ControllerError::MissingNamespace("PersistentVolumeClaim"))?
        .to_string();
    let name = pvc
        .name()
        .ok_or(ControllerError::MissingName("PersistentVolumeClaim"))?
        .to_string();
    let uid = pvc.object_meta().as_ref().and_then(|meta| meta.uid.clone());
    Ok((namespace, name, uid))
}

pub(crate) fn provisioner_config<'a>(
    config: &'a ControllerManagerConfig,
    provisioner: &str,
) -> Option<&'a ProvisionerConfig> {
    config.csi.provisioners.get(provisioner)
}

pub(crate) fn storage_class_provisioner(
    storage_class: &StorageClass,
) -> Result<(String, HashMap<String, String>), ControllerError> {
    let name = storage_class
        .name()
        .ok_or(ControllerError::MissingName("StorageClass"))?
        .to_string();
    let spec = storage_class
        .spec
        .as_ref()
        .ok_or_else(|| ControllerError::MissingStorageClassSpec { name: name.clone() })?;
    if spec.provisioner.is_empty() {
        return Err(ControllerError::MissingProvisioner { name });
    }
    Ok((spec.provisioner.clone(), spec.parameters.clone()))
}

pub(crate) fn reclaim_policy_from_storage_class(
    storage_class: &StorageClass,
) -> Result<String, ControllerError> {
    let name = storage_class
        .name()
        .ok_or(ControllerError::MissingName("StorageClass"))?
        .to_string();
    let spec = storage_class
        .spec
        .as_ref()
        .ok_or_else(|| ControllerError::MissingStorageClassSpec { name: name.clone() })?;
    Ok(spec
        .reclaim_policy
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| RECLAIM_POLICY_DELETE.to_string()))
}

pub(crate) fn claim_access_modes(
    namespace: &str,
    name: &str,
    access_modes: &[String],
) -> Result<Vec<CsiAccessMode>, ControllerError> {
    if access_modes.is_empty() {
        return Err(ControllerError::MissingAccessModes {
            namespace: namespace.to_string(),
            name: name.to_string(),
        });
    }

    let mut resolved = Vec::with_capacity(access_modes.len());
    for mode in access_modes {
        let mode = match mode.as_str() {
            READ_ONLY_MANY => CsiAccessMode::ReadOnlyMany,
            READ_WRITE_ONCE => CsiAccessMode::ReadWriteOnce,
            READ_WRITE_MANY => CsiAccessMode::ReadWriteMany,
            _ => {
                return Err(ControllerError::UnsupportedAccessMode {
                    namespace: namespace.to_string(),
                    name: name.to_string(),
                    mode: mode.clone(),
                });
            }
        };
        if !resolved.contains(&mode) {
            resolved.push(mode);
        }
    }

    Ok(resolved)
}

pub(crate) fn claim_access_type(
    namespace: &str,
    name: &str,
    volume_mode: Option<&str>,
) -> Result<CsiAccessType, ControllerError> {
    match volume_mode.unwrap_or(DEFAULT_VOLUME_MODE) {
        DEFAULT_VOLUME_MODE => Ok(CsiAccessType::Block),
        mode => Err(ControllerError::UnsupportedVolumeMode {
            namespace: namespace.to_string(),
            name: name.to_string(),
            mode: mode.to_string(),
        }),
    }
}

pub(crate) fn dynamic_volume_name(namespace: &str, name: &str, uid: Option<&str>) -> String {
    let base = uid
        .map(|uid| format!("pvc-{uid}"))
        .unwrap_or_else(|| format!("pvc-{namespace}-{name}"));
    sanitize_resource_name(&base)
}

pub(crate) fn managed_pv_label_selector() -> String {
    format!("{MANAGED_LABEL}={MANAGED_LABEL_VALUE}")
}

pub(crate) fn is_managed_pv(pv: &PersistentVolume) -> bool {
    pv.object_meta()
        .as_ref()
        .and_then(|meta| meta.labels.get(MANAGED_LABEL))
        .is_some_and(|value| value == MANAGED_LABEL_VALUE)
}

pub(crate) fn should_delete_backing_volume(pv: &PersistentVolume) -> Result<bool, ControllerError> {
    let name = pv
        .name()
        .ok_or(ControllerError::MissingName("PersistentVolume"))?
        .to_string();
    let spec = pv
        .spec
        .as_ref()
        .ok_or_else(|| ControllerError::MissingPersistentVolumeSpec { name })?;
    Ok(spec
        .persistent_volume_reclaim_policy
        .as_deref()
        .unwrap_or(RECLAIM_POLICY_DELETE)
        == RECLAIM_POLICY_DELETE)
}

pub(crate) fn pv_provisioner(pv: &PersistentVolume) -> Result<String, ControllerError> {
    if let Some(provisioner) = pv
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.annotations.get(PROVISIONER_ANNOTATION))
        .filter(|value| !value.is_empty())
    {
        return Ok(provisioner.clone());
    }

    let name = pv
        .name()
        .ok_or(ControllerError::MissingName("PersistentVolume"))?
        .to_string();
    let spec = pv
        .spec
        .as_ref()
        .ok_or_else(|| ControllerError::MissingPersistentVolumeSpec { name: name.clone() })?;
    let csi = spec
        .csi
        .as_ref()
        .ok_or_else(|| ControllerError::MissingPersistentVolumeCsi { name: name.clone() })?;
    if csi.driver.is_empty() {
        return Err(ControllerError::MissingProvisioner { name });
    }
    Ok(csi.driver.clone())
}

pub(crate) fn existing_pv_matches_claim(
    pv: &PersistentVolume,
    namespace: &str,
    claim: &str,
    storage_class_name: &str,
) -> Result<bool, ControllerError> {
    let name = pv
        .name()
        .ok_or(ControllerError::MissingName("PersistentVolume"))?
        .to_string();
    let spec = pv
        .spec
        .as_ref()
        .ok_or_else(|| ControllerError::MissingPersistentVolumeSpec { name: name.clone() })?;

    let Some(claim_ref) = spec.claim_ref.as_ref() else {
        return Ok(false);
    };

    Ok(is_managed_pv(pv)
        && claim_ref.namespace == namespace
        && claim_ref.name == claim
        && spec.storage_class_name.as_deref() == Some(storage_class_name))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_persistent_volume(
    pv_name: &str,
    claim_namespace: &str,
    claim_name: &str,
    storage_class_name: String,
    provisioner: String,
    reclaim_policy: String,
    access_modes: Vec<String>,
    volume_mode: Option<String>,
    volume_handle: String,
) -> PersistentVolume {
    PersistentVolume {
        object_meta: Some(ObjectMeta {
            name: Some(pv_name.to_string()),
            labels: HashMap::from([(MANAGED_LABEL.to_string(), MANAGED_LABEL_VALUE.to_string())]),
            annotations: HashMap::from([
                (PROVISIONER_ANNOTATION.to_string(), provisioner.clone()),
                (
                    CLAIM_NAMESPACE_ANNOTATION.to_string(),
                    claim_namespace.to_string(),
                ),
                (CLAIM_NAME_ANNOTATION.to_string(), claim_name.to_string()),
            ]),
            ..Default::default()
        }),
        spec: Some(PersistentVolumeSpec {
            access_modes,
            persistent_volume_reclaim_policy: Some(reclaim_policy),
            storage_class_name: Some(storage_class_name),
            volume_mode,
            csi: Some(CsiPersistentVolumeSource {
                driver: provisioner,
                controller_expand_secret_ref: None,
                controller_publish_secret_ref: None,
                node_expand_secret_ref: None,
                node_publish_secret_ref: None,
                node_stage_secret_ref: None,
                read_only: false,
                volume_handle,
            }),
            claim_ref: Some(PersistentVolumeClaimReference {
                name: claim_name.to_string(),
                namespace: claim_namespace.to_string(),
            }),
        }),
        ..Default::default()
    }
}

fn sanitize_resource_name(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    let mut previous_was_dash = false;

    for ch in input.chars() {
        let lowered = ch.to_ascii_lowercase();
        if lowered.is_ascii_lowercase() || lowered.is_ascii_digit() {
            sanitized.push(lowered);
            previous_was_dash = false;
        } else if !previous_was_dash && !sanitized.is_empty() {
            sanitized.push('-');
            previous_was_dash = true;
        }
    }

    while sanitized.ends_with('-') {
        sanitized.pop();
    }

    if sanitized.is_empty() {
        sanitized.push_str("pvc-volume");
    }

    if sanitized.len() > 253 {
        sanitized.truncate(253);
        while sanitized.ends_with('-') {
            sanitized.pop();
        }
    }

    if sanitized.is_empty() {
        "pvc-volume".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MANAGED_LABEL, MANAGED_LABEL_VALUE, build_persistent_volume, claim_access_modes,
        claim_access_type, dynamic_volume_name, is_managed_pv, should_delete_backing_volume,
    };
    use tugboat_csi_operator::{CsiAccessMode, CsiAccessType};

    #[test]
    fn dynamic_volume_name_prefers_uid() {
        assert_eq!(
            dynamic_volume_name("default", "claim-a", Some("1234-uid")),
            "pvc-1234-uid"
        );
    }

    #[test]
    fn dynamic_volume_name_sanitizes_namespace_and_name() {
        assert_eq!(
            dynamic_volume_name("default", "Claim_Name", None),
            "pvc-default-claim-name"
        );
    }

    #[test]
    fn claim_access_modes_map_supported_values() {
        let modes = claim_access_modes(
            "default",
            "claim-a",
            &["ReadWriteOnce".to_string(), "ReadOnlyMany".to_string()],
        )
        .unwrap();

        assert_eq!(
            modes,
            vec![CsiAccessMode::ReadWriteOnce, CsiAccessMode::ReadOnlyMany]
        );
    }

    #[test]
    fn claim_access_type_defaults_to_block() {
        assert_eq!(
            claim_access_type("default", "claim-a", None).unwrap(),
            CsiAccessType::Block
        );
    }

    #[test]
    fn managed_persistent_volume_is_labeled_and_deletes_by_default() {
        let pv = build_persistent_volume(
            "pv-1",
            "default",
            "claim-a",
            "fast".to_string(),
            "example.csi.driver".to_string(),
            "Delete".to_string(),
            vec!["ReadWriteOnce".to_string()],
            Some("Block".to_string()),
            "volume-1".to_string(),
        );

        assert!(is_managed_pv(&pv));
        assert_eq!(
            pv.object_meta
                .as_ref()
                .unwrap()
                .labels
                .get(MANAGED_LABEL)
                .map(String::as_str),
            Some(MANAGED_LABEL_VALUE)
        );
        assert!(should_delete_backing_volume(&pv).unwrap());
    }

    #[test]
    fn retain_policy_skips_backing_volume_deletion() {
        let pv = build_persistent_volume(
            "pv-1",
            "default",
            "claim-a",
            "fast".to_string(),
            "example.csi.driver".to_string(),
            "Retain".to_string(),
            vec!["ReadWriteOnce".to_string()],
            Some("Block".to_string()),
            "volume-1".to_string(),
        );

        assert!(!should_delete_backing_volume(&pv).unwrap());
    }
}
