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

use crate::framework::{FilterPlugin, FilterResult, SchedulingContext};
use tugboat_resources::manifests::core::v1::Node;

const READ_WRITE_MANY: &str = "ReadWriteMany";

/// Rejects nodes when any PVC used by the Ship is backed by a PersistentVolume
/// that does not support `ReadWriteMany` access. Ships with RWO-only volumes
/// cannot be live-migrated, so scheduling them to a node would prevent future
/// migration and is therefore rejected here to surface the issue early.
///
/// If a Ship has no PVC-backed volumes, every node is accepted.
pub struct StorageFitFilter;

impl FilterPlugin for StorageFitFilter {
    fn name(&self) -> &str {
        "StorageFit"
    }

    fn filter(&self, ctx: &SchedulingContext, _node: &Node) -> FilterResult {
        for pv in ctx.ship_bound_persistent_volumes() {
            let Some(spec) = pv.spec.as_ref() else {
                continue;
            };

            let has_rwx = spec.access_modes.iter().any(|mode| mode == READ_WRITE_MANY);

            if !has_rwx {
                let pv_name = pv
                    .object_meta
                    .as_ref()
                    .and_then(|m| m.name.as_deref())
                    .unwrap_or("<unknown>");
                return FilterResult::Reject(format!(
                    "PersistentVolume '{pv_name}' does not support '{READ_WRITE_MANY}'; \
                     live migration requires shared storage on all volumes"
                ));
            }
        }

        FilterResult::Accept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::SchedulingContext;
    use tugboat_resources::manifests::core::v1::{
        PersistentVolume, PersistentVolumeClaimSpec, PersistentVolumeSpec, Ship, ShipClass,
        ShipSpec, ShipVolume,
    };
    use tugboat_resources::manifests::core::v1::{
        PersistentVolumeClaim, PersistentVolumeClaimVolumeSource,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    fn make_pv(name: &str, access_modes: Vec<&str>) -> PersistentVolume {
        PersistentVolume {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            }),
            spec: Some(PersistentVolumeSpec {
                access_modes: access_modes.into_iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn make_pvc(name: &str, namespace: &str, pv_name: &str) -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            }),
            spec: Some(PersistentVolumeClaimSpec {
                volume_name: Some(pv_name.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn make_ctx(
        pvcs: Vec<PersistentVolumeClaim>,
        pvs: Vec<PersistentVolume>,
        volumes: Vec<ShipVolume>,
    ) -> SchedulingContext {
        SchedulingContext {
            ship: Ship {
                object_meta: Some(ObjectMeta {
                    namespace: Some("default".to_string()),
                    ..Default::default()
                }),
                spec: Some(ShipSpec {
                    volumes,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ship_class: ShipClass::default(),
            all_cluster_network_classes: Vec::new(),
            all_network_classes: Vec::new(),
            all_ships: Vec::new(),
            all_ship_classes: Vec::new(),
            all_persistent_volume_claims: pvcs,
            all_persistent_volumes: pvs,
        }
    }

    #[test]
    fn accepts_ship_with_rwx_volume() {
        let pv = make_pv("pv-1", vec![READ_WRITE_MANY]);
        let pvc = make_pvc("pvc-1", "default", "pv-1");
        let volume = ShipVolume {
            name: "data".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "pvc-1".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ctx = make_ctx(vec![pvc], vec![pv], vec![volume]);
        let filter = StorageFitFilter;
        assert!(matches!(
            filter.filter(&ctx, &Node::default()),
            FilterResult::Accept
        ));
    }

    #[test]
    fn rejects_ship_with_rwo_only_volume() {
        let pv = make_pv("pv-1", vec!["ReadWriteOnce"]);
        let pvc = make_pvc("pvc-1", "default", "pv-1");
        let volume = ShipVolume {
            name: "data".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "pvc-1".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let ctx = make_ctx(vec![pvc], vec![pv], vec![volume]);
        let filter = StorageFitFilter;
        assert!(matches!(
            filter.filter(&ctx, &Node::default()),
            FilterResult::Reject(_)
        ));
    }

    #[test]
    fn accepts_ship_with_no_volumes() {
        let ctx = make_ctx(Vec::new(), Vec::new(), Vec::new());
        let filter = StorageFitFilter;
        assert!(matches!(
            filter.filter(&ctx, &Node::default()),
            FilterResult::Accept
        ));
    }
}
