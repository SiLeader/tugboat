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
pub(crate) mod delete;
pub(crate) mod modify;

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

/// Fingerprint covering only the set of volume claim references.
pub(super) fn volume_claims_fingerprint(spec: &ShipSpec) -> Result<String, serde_json::Error> {
    sha256_fingerprint(&spec.volume_claim_ref)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tugboat_resources::manifests::core::v1::{ShipSpec, Toleration};

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
            volume_claims_fingerprint(&a).unwrap(),
            volume_claims_fingerprint(&b).unwrap(),
            "scheduling-only field changes must not alter the volume fingerprint"
        );
    }

    #[test]
    fn image_change_alters_spec_fingerprint() {
        let a = base_spec();
        let mut b = base_spec();
        b.image = "registry.example.com/vm:v2".to_string();
        assert_ne!(spec_fingerprint(&a).unwrap(), spec_fingerprint(&b).unwrap());
        // volume fingerprint should stay the same
        assert_eq!(
            volume_claims_fingerprint(&a).unwrap(),
            volume_claims_fingerprint(&b).unwrap()
        );
    }

    #[test]
    fn volume_claim_change_alters_volume_fingerprint_only() {
        let a = base_spec();
        let mut b = base_spec();
        b.volume_claim_ref.push(
            tugboat_resources::manifests::core::v1::ShipVolumeClaimReference {
                name: "pvc-1".to_string(),
            },
        );
        assert_ne!(
            volume_claims_fingerprint(&a).unwrap(),
            volume_claims_fingerprint(&b).unwrap()
        );
        // spec fingerprint should stay the same
        assert_eq!(spec_fingerprint(&a).unwrap(), spec_fingerprint(&b).unwrap());
    }
}
