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

pub(crate) mod add;
pub(crate) mod add_helpers;
pub(crate) mod delete;
pub(crate) mod modify;

use crate::reconciler::error::ReconcileError;
use crate::reconciler::volume::normalized_ship_volumes;
use serde::Serialize;
use tugboat_resources::manifests::core::v1::ShipSpec;

fn sha256_fingerprint<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    use sha2::Digest;
    let json = serde_json::to_string(value)?;
    let hash = sha2::Sha256::digest(json.as_bytes());
    Ok(format!("{hash:x}"))
}

/// Fingerprint covering non-volume runtime fields: image, ship_class, uefi,
/// network_class_ref. Changes to any of these require a VM recreate.
pub(super) fn spec_fingerprint(spec: &ShipSpec) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct SpecFields<'a> {
        image: &'a str,
        ship_class: &'a str,
        network_class_ref:
            &'a [tugboat_resources::manifests::core::v1::ShipNetworkClassReference],
        uefi: &'a Option<tugboat_resources::manifests::core::v1::ShipUefi>,
    }

    sha256_fingerprint(&SpecFields {
        image: &spec.image,
        ship_class: &spec.ship_class,
        network_class_ref: &spec.network_class_ref,
        uefi: &spec.uefi,
    })
}

/// Fingerprint covering only the PersistentVolumeClaim volume references.
/// Changes to PVC bindings require a full VM recreate.
pub(super) fn pvc_volume_fingerprint(spec: &ShipSpec) -> Result<String, ReconcileError> {
    use crate::reconciler::volume::NormalizedVolumeSource;
    let normalized: Vec<_> = normalized_ship_volumes(spec)?
        .into_iter()
        .filter(|v| {
            matches!(
                v.source,
                NormalizedVolumeSource::PersistentVolumeClaim { .. }
            )
        })
        .collect();
    Ok(sha256_fingerprint(&normalized)?)
}

/// Fingerprint covering only the materialized (ConfigMap / Secret) volume references.
/// Changes to these can be applied in-place by re-materializing files on the host.
pub(super) fn materialized_volume_fingerprint(spec: &ShipSpec) -> Result<String, ReconcileError> {
    use crate::reconciler::volume::NormalizedVolumeSource;
    let normalized: Vec<_> = normalized_ship_volumes(spec)?
        .into_iter()
        .filter(|v| {
            matches!(
                v.source,
                NormalizedVolumeSource::ConfigMap { .. } | NormalizedVolumeSource::Secret { .. }
            )
        })
        .collect();
    Ok(sha256_fingerprint(&normalized)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tugboat_resources::manifests::core::v1::{
        ConfigMapVolumeSource, ShipSpec, ShipVolume, Toleration,
    };

    fn base_spec() -> ShipSpec {
        ShipSpec {
            image: "registry.example.com/vm:latest".to_string(),
            ship_class: "small".to_string(),
            node_name: None,
            network_class_ref: vec![],
            uefi: None,
            tolerations: vec![],
            scheduler_name: None,
            volume_claim_ref: vec![],
            volumes: vec![],
        }
    }

    #[test]
    fn scheduling_only_changes_produce_same_fingerprints() {
        let a = base_spec();
        let mut b = base_spec();
        b.tolerations.push(Toleration {
            key: Some("key".to_string()),
            operator: "Equal".to_string(),
            value: Some("value".to_string()),
            effect: "NoSchedule".to_string(),
            toleration_seconds: None,
        });
        b.scheduler_name = Some("custom-scheduler".to_string());
        b.node_name = Some("node-1".to_string());
        assert_eq!(
            spec_fingerprint(&a).unwrap(),
            spec_fingerprint(&b).unwrap(),
            "scheduling-only field changes must not alter the spec fingerprint"
        );
        assert_eq!(
            pvc_volume_fingerprint(&a).unwrap(),
            pvc_volume_fingerprint(&b).unwrap(),
            "scheduling-only field changes must not alter the pvc volume fingerprint"
        );
        assert_eq!(
            materialized_volume_fingerprint(&a).unwrap(),
            materialized_volume_fingerprint(&b).unwrap(),
            "scheduling-only field changes must not alter the materialized volume fingerprint"
        );
    }

    #[test]
    fn image_change_alters_spec_fingerprint() {
        let a = base_spec();
        let mut b = base_spec();
        b.image = "registry.example.com/vm:v2".to_string();
        assert_ne!(spec_fingerprint(&a).unwrap(), spec_fingerprint(&b).unwrap());
        // volume fingerprints should stay the same
        assert_eq!(
            pvc_volume_fingerprint(&a).unwrap(),
            pvc_volume_fingerprint(&b).unwrap()
        );
        assert_eq!(
            materialized_volume_fingerprint(&a).unwrap(),
            materialized_volume_fingerprint(&b).unwrap()
        );
    }

    #[test]
    fn pvc_change_alters_pvc_fingerprint_not_materialized() {
        let a = base_spec();
        let mut b = base_spec();
        b.volume_claim_ref.push(
            tugboat_resources::manifests::core::v1::ShipVolumeClaimReference {
                name: "pvc-1".to_string(),
            },
        );
        assert_ne!(
            pvc_volume_fingerprint(&a).unwrap(),
            pvc_volume_fingerprint(&b).unwrap(),
            "pvc_volume_fingerprint must change when PVC refs change"
        );
        assert_eq!(
            materialized_volume_fingerprint(&a).unwrap(),
            materialized_volume_fingerprint(&b).unwrap(),
            "materialized_volume_fingerprint must not change when only PVC refs change"
        );
        // spec fingerprint should stay the same
        assert_eq!(spec_fingerprint(&a).unwrap(), spec_fingerprint(&b).unwrap());
    }

    #[test]
    fn configmap_change_alters_materialized_fingerprint_not_pvc() {
        let a = base_spec();
        let mut b = base_spec();
        b.volumes.push(ShipVolume {
            name: "config".to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: "app-config".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_ne!(
            materialized_volume_fingerprint(&a).unwrap(),
            materialized_volume_fingerprint(&b).unwrap(),
            "materialized_volume_fingerprint must change when ConfigMap volumes change"
        );
        assert_eq!(
            pvc_volume_fingerprint(&a).unwrap(),
            pvc_volume_fingerprint(&b).unwrap(),
            "pvc_volume_fingerprint must not change when only ConfigMap volumes change"
        );
        assert_eq!(spec_fingerprint(&a).unwrap(), spec_fingerprint(&b).unwrap());
    }

    #[test]
    fn secret_change_alters_materialized_fingerprint_not_pvc() {
        let a = base_spec();
        let mut b = base_spec();
        b.volumes.push(ShipVolume {
            name: "secrets".to_string(),
            secret: Some(tugboat_resources::manifests::core::v1::SecretVolumeSource {
                secret_name: "app-secret".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_ne!(
            materialized_volume_fingerprint(&a).unwrap(),
            materialized_volume_fingerprint(&b).unwrap(),
            "materialized_volume_fingerprint must change when Secret volumes change"
        );
        assert_eq!(
            pvc_volume_fingerprint(&a).unwrap(),
            pvc_volume_fingerprint(&b).unwrap(),
            "pvc_volume_fingerprint must not change when only Secret volumes change"
        );
        assert_eq!(spec_fingerprint(&a).unwrap(), spec_fingerprint(&b).unwrap());
    }

    #[test]
    fn mixed_change_alters_both_volume_fingerprints() {
        let a = base_spec();
        let mut b = base_spec();
        b.volume_claim_ref.push(
            tugboat_resources::manifests::core::v1::ShipVolumeClaimReference {
                name: "pvc-1".to_string(),
            },
        );
        b.volumes.push(ShipVolume {
            name: "config".to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: "app-config".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_ne!(
            pvc_volume_fingerprint(&a).unwrap(),
            pvc_volume_fingerprint(&b).unwrap(),
            "pvc_volume_fingerprint must change when PVC refs change"
        );
        assert_ne!(
            materialized_volume_fingerprint(&a).unwrap(),
            materialized_volume_fingerprint(&b).unwrap(),
            "materialized_volume_fingerprint must change when ConfigMap volumes change"
        );
    }
}
