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
/// that does not support `ReadWriteMany` access, but only for ShipClasses that
/// explicitly opt into live-migration settings. This keeps non-migratable
/// workloads free to use local RWO storage while still surfacing shared-storage
/// requirements early for workloads that intend to migrate.
///
/// If a Ship has no PVC-backed volumes, every node is accepted.
pub struct StorageFitFilter;

impl FilterPlugin for StorageFitFilter {
    fn name(&self) -> &str {
        "StorageFit"
    }

    fn filter(&self, ctx: &SchedulingContext, _node: &Node) -> FilterResult {
        if !ship_requires_shared_storage(ctx) {
            return FilterResult::Accept;
        }

        for (pvc, pv) in bound_pvc_pv_pairs(ctx) {
            let pvc_has_rwx = pvc
                .spec
                .as_ref()
                .map(|spec| spec.access_modes.iter().any(|mode| mode == READ_WRITE_MANY))
                .unwrap_or(false);

            if !pvc_has_rwx {
                let pvc_name = pvc
                    .object_meta
                    .as_ref()
                    .and_then(|m| m.name.as_deref())
                    .unwrap_or("<unknown>");
                return FilterResult::Reject(format!(
                    "PersistentVolumeClaim '{pvc_name}' does not support '{READ_WRITE_MANY}'; \
                     shared storage is required on all volumes because the ShipClass has migration enabled"
                ));
            }

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
                     shared storage is required on all volumes because the ShipClass has migration enabled"
                ));
            }
        }

        FilterResult::Accept
    }
}

fn ship_requires_shared_storage(ctx: &SchedulingContext) -> bool {
    ctx.ship_class
        .spec
        .as_ref()
        .and_then(|spec| spec.migration.as_ref())
        .is_some()
}

fn bound_pvc_pv_pairs(
    ctx: &SchedulingContext,
) -> Vec<(
    &tugboat_resources::manifests::core::v1::PersistentVolumeClaim,
    &tugboat_resources::manifests::core::v1::PersistentVolume,
)> {
    let namespace = ctx.ship_namespace();
    let Some(spec) = ctx.ship.spec.as_ref() else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for volume in &spec.volumes {
        let Some(pvc_source) = volume.persistent_volume_claim.as_ref() else {
            continue;
        };
        let claim_name = pvc_source.claim_name.as_str();
        let Some(pvc) = ctx.all_persistent_volume_claims.iter().find(|pvc| {
            let meta = pvc.object_meta.as_ref();
            meta.and_then(|m| m.name.as_deref()) == Some(claim_name)
                && meta.and_then(|m| m.namespace.as_deref()) == Some(namespace)
        }) else {
            continue;
        };

        let pv_name = pvc
            .spec
            .as_ref()
            .and_then(|s| s.volume_name.as_deref())
            .unwrap_or("");
        if pv_name.is_empty() {
            continue;
        }

        if let Some(pv) = ctx
            .all_persistent_volumes
            .iter()
            .find(|pv| pv.object_meta.as_ref().and_then(|m| m.name.as_deref()) == Some(pv_name))
        {
            result.push((pvc, pv));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::SchedulingContext;
    use tugboat_resources::manifests::core::v1::{
        MigrationSpec, PersistentVolume, PersistentVolumeClaimSpec, PersistentVolumeSpec, Ship,
        ShipClass, ShipClassSpec, ShipSpec, ShipVolume,
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

    fn make_pvc(
        name: &str,
        namespace: &str,
        pv_name: &str,
        access_modes: Vec<&str>,
    ) -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            }),
            spec: Some(PersistentVolumeClaimSpec {
                volume_name: Some(pv_name.to_string()),
                access_modes: access_modes.into_iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn make_ctx(
        pvcs: Vec<PersistentVolumeClaim>,
        pvs: Vec<PersistentVolume>,
        volumes: Vec<ShipVolume>,
        migration_enabled: bool,
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
            ship_class: ShipClass {
                spec: Some(ShipClassSpec {
                    migration: migration_enabled.then(MigrationSpec::default),
                    ..Default::default()
                }),
                ..Default::default()
            },
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
        let pvc = make_pvc("pvc-1", "default", "pv-1", vec![READ_WRITE_MANY]);
        let volume = ShipVolume {
            name: "data".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "pvc-1".to_string(),
            }),
            ..Default::default()
        };
        let ctx = make_ctx(vec![pvc], vec![pv], vec![volume], true);
        let filter = StorageFitFilter;
        assert!(matches!(
            filter.filter(&ctx, &Node::default()),
            FilterResult::Accept
        ));
    }

    #[test]
    fn rejects_ship_with_rwo_only_volume() {
        let pv = make_pv("pv-1", vec!["ReadWriteOnce"]);
        let pvc = make_pvc("pvc-1", "default", "pv-1", vec![READ_WRITE_MANY]);
        let volume = ShipVolume {
            name: "data".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "pvc-1".to_string(),
            }),
            ..Default::default()
        };
        let ctx = make_ctx(vec![pvc], vec![pv], vec![volume], true);
        let filter = StorageFitFilter;
        assert!(matches!(
            filter.filter(&ctx, &Node::default()),
            FilterResult::Reject(_)
        ));
    }

    #[test]
    fn rejects_ship_with_pvc_missing_rwx() {
        let pv = make_pv("pv-1", vec![READ_WRITE_MANY]);
        let pvc = make_pvc("pvc-1", "default", "pv-1", vec!["ReadWriteOnce"]);
        let volume = ShipVolume {
            name: "data".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "pvc-1".to_string(),
            }),
            ..Default::default()
        };
        let ctx = make_ctx(vec![pvc], vec![pv], vec![volume], true);
        let filter = StorageFitFilter;
        assert!(matches!(
            filter.filter(&ctx, &Node::default()),
            FilterResult::Reject(_)
        ));
    }

    #[test]
    fn accepts_ship_with_no_volumes() {
        let ctx = make_ctx(Vec::new(), Vec::new(), Vec::new(), true);
        let filter = StorageFitFilter;
        assert!(matches!(
            filter.filter(&ctx, &Node::default()),
            FilterResult::Accept
        ));
    }

    #[test]
    fn accepts_rwo_volume_when_migration_is_not_configured() {
        let pv = make_pv("pv-1", vec!["ReadWriteOnce"]);
        let pvc = make_pvc("pvc-1", "default", "pv-1", vec!["ReadWriteOnce"]);
        let volume = ShipVolume {
            name: "data".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "pvc-1".to_string(),
            }),
            ..Default::default()
        };
        let ctx = make_ctx(vec![pvc], vec![pv], vec![volume], false);
        let filter = StorageFitFilter;
        assert!(matches!(
            filter.filter(&ctx, &Node::default()),
            FilterResult::Accept
        ));
    }
}
