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

//! Fanout controller: when a `ShipSnapshot` has `spec.include_volumes`,
//! ensure a `VolumeSnapshot` exists for each PVC the referenced Ship
//! mounts, set owner references so the snapshots are garbage-collected
//! with the parent, and reflect their readiness in
//! `ShipSnapshot.status.volume_snapshots`.

use crate::base::TugboatController;
use crate::error::ControllerError;
use tugboat_client::runtime::{Action, Controller, ReconcileEvent, Reconciler};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::{
    Ship, ShipSnapshot, ShipSnapshotStatus, ShipSnapshotVolumeRef,
};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference};
use tugboat_resources::manifests::snapshot::v1::{
    VolumeSnapshot, VolumeSnapshotSource, VolumeSnapshotSpec,
};
use tugboat_resources::{ObjectMetaResource, Resource};

#[derive(Clone)]
struct ShipSnapshotVolumesReconciler {
    client: TugboatClient,
}

pub(crate) struct ShipSnapshotVolumesController {
    controller: Controller<ShipSnapshot>,
    reconciler: ShipSnapshotVolumesReconciler,
}

impl ShipSnapshotVolumesController {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: ShipSnapshotVolumesReconciler { client },
        }
    }
}

#[async_trait::async_trait]
impl TugboatController for ShipSnapshotVolumesController {
    fn name(&self) -> &str {
        "ship-snapshot-volumes"
    }

    async fn setup(&mut self) {}

    async fn run(&self) {
        self.controller.clone().run(self.reconciler.clone()).await;
    }
}

#[async_trait::async_trait]
impl Reconciler<ShipSnapshot> for ShipSnapshotVolumesReconciler {
    type Error = ControllerError;

    async fn reconcile(&self, event: ReconcileEvent<ShipSnapshot>) -> Result<Action, Self::Error> {
        match event {
            ReconcileEvent::Applied(snapshot) => self.reconcile_applied(snapshot).await,
            ReconcileEvent::Deleted(_) => Ok(Action::await_change()),
        }
    }
}

impl ShipSnapshotVolumesReconciler {
    async fn reconcile_applied(&self, snapshot: ShipSnapshot) -> Result<Action, ControllerError> {
        if snapshot.deletion_timestamp().is_some() {
            return Ok(Action::await_change());
        }
        let Some(spec) = snapshot.spec.as_ref() else {
            return Ok(Action::await_change());
        };
        if !spec.include_volumes.unwrap_or(false) {
            return Ok(Action::await_change());
        }
        let namespace = snapshot
            .namespace()
            .ok_or(ControllerError::MissingNamespace("ShipSnapshot"))?
            .to_string();
        let name = snapshot
            .name()
            .ok_or(ControllerError::MissingName("ShipSnapshot"))?
            .to_string();
        let uid = snapshot
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.uid.clone())
            .unwrap_or_default();

        let ship_api: Api<Ship> = Api::namespaced(self.client.clone(), &namespace);
        let Some(ship) = ship_api.get(&spec.ship_name).await? else {
            return Ok(Action::await_change());
        };
        let pvc_names = ship_pvc_names(&ship);

        let snapshot_api: Api<VolumeSnapshot> = Api::namespaced(self.client.clone(), &namespace);
        let mut refs = Vec::with_capacity(pvc_names.len());
        for pvc_name in &pvc_names {
            let volume_snapshot_name = derived_volume_snapshot_name(&name, pvc_name);
            let desired = build_volume_snapshot(
                &volume_snapshot_name,
                &namespace,
                &name,
                &uid,
                pvc_name,
                spec.volume_snapshot_class_name.clone(),
            );
            let ready = match snapshot_api.create(desired.clone()).await {
                Ok(created) => snapshot_ready_to_use(&created),
                Err(tugboat_client::Error::Api(status)) if status.code == 409 => {
                    match snapshot_api.get(&volume_snapshot_name).await? {
                        Some(existing) => snapshot_ready_to_use(&existing),
                        None => None,
                    }
                }
                Err(err) => return Err(err.into()),
            };
            refs.push(ShipSnapshotVolumeRef {
                pvc_name: pvc_name.clone(),
                volume_snapshot_name,
                ready_to_use: ready,
            });
        }

        if status_volume_refs_changed(snapshot.status.as_ref(), &refs) {
            patch_status_volume_refs(&self.client, &namespace, &name, &snapshot, refs).await?;
        }
        Ok(Action::await_change())
    }
}

fn ship_pvc_names(ship: &Ship) -> Vec<String> {
    let Some(spec) = ship.spec.as_ref() else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for vc in &spec.volume_claim_ref {
        if !vc.name.is_empty() && !names.contains(&vc.name) {
            names.push(vc.name.clone());
        }
    }
    for volume in &spec.volumes {
        if let Some(pvc) = volume.persistent_volume_claim.as_ref()
            && !pvc.claim_name.is_empty()
            && !names.contains(&pvc.claim_name)
        {
            names.push(pvc.claim_name.clone());
        }
    }
    names
}

pub(crate) fn derived_volume_snapshot_name(snapshot_name: &str, pvc_name: &str) -> String {
    let combined = format!("{snapshot_name}-{pvc_name}");
    sanitize_resource_name(&combined)
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
        return "shipsnapshot-volume".to_string();
    }
    if sanitized.len() > 253 {
        sanitized.truncate(253);
        while sanitized.ends_with('-') {
            sanitized.pop();
        }
    }
    sanitized
}

fn build_volume_snapshot(
    volume_snapshot_name: &str,
    namespace: &str,
    owner_name: &str,
    owner_uid: &str,
    pvc_name: &str,
    snapshot_class_name: Option<String>,
) -> VolumeSnapshot {
    VolumeSnapshot {
        type_meta: Some(VolumeSnapshot::type_meta()),
        object_meta: Some(ObjectMeta {
            name: Some(volume_snapshot_name.to_string()),
            namespace: Some(namespace.to_string()),
            owner_references: vec![OwnerReference {
                api_version: "snapshot/v1".to_string(),
                kind: "ShipSnapshot".to_string(),
                name: owner_name.to_string(),
                uid: owner_uid.to_string(),
                controller: Some(true),
            }],
            ..Default::default()
        }),
        spec: Some(VolumeSnapshotSpec {
            source: Some(VolumeSnapshotSource {
                persistent_volume_claim_name: Some(pvc_name.to_string()),
                volume_snapshot_content_name: None,
            }),
            volume_snapshot_class_name: snapshot_class_name,
        }),
        status: None,
    }
}

fn snapshot_ready_to_use(snapshot: &VolumeSnapshot) -> Option<bool> {
    snapshot
        .status
        .as_ref()
        .and_then(|status| status.ready_to_use)
}

fn status_volume_refs_changed(
    existing: Option<&ShipSnapshotStatus>,
    desired: &[ShipSnapshotVolumeRef],
) -> bool {
    let existing_refs = existing.map(|status| status.volume_snapshots.as_slice());
    !matches!(existing_refs, Some(refs) if refs == desired)
}

async fn patch_status_volume_refs(
    client: &TugboatClient,
    namespace: &str,
    name: &str,
    current: &ShipSnapshot,
    refs: Vec<ShipSnapshotVolumeRef>,
) -> Result<(), ControllerError> {
    let mut updated = current.clone();
    let status = updated
        .status
        .get_or_insert_with(ShipSnapshotStatus::default);
    // Preserve any existing phase/handle; only the volume_snapshots view
    // is the controller's responsibility.
    status.volume_snapshots = refs;
    if status.phase.is_empty() {
        status.phase = "Pending".to_string();
    }
    let api: Api<ShipSnapshot> = Api::namespaced(client.clone(), namespace);
    api.replace_status(name, updated).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tugboat_resources::manifests::core::v1::{
        PersistentVolumeClaimVolumeSource, ShipSnapshotSpec, ShipSpec, ShipVolume,
        ShipVolumeClaimReference,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    fn ship_with_volumes() -> Ship {
        Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-a".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            spec: Some(ShipSpec {
                image: "img".to_string(),
                ship_class: "small".to_string(),
                volume_claim_ref: vec![ShipVolumeClaimReference {
                    name: "data".to_string(),
                }],
                volumes: vec![ShipVolume {
                    name: "extra".to_string(),
                    persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                        claim_name: "logs".to_string(),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn ship_pvc_names_dedupes_and_orders_inputs() {
        let mut ship = ship_with_volumes();
        // duplicate the data claim in both volume_claim_ref and volumes
        ship.spec.as_mut().unwrap().volumes.push(ShipVolume {
            name: "again".to_string(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: "data".to_string(),
            }),
            ..Default::default()
        });
        let names = ship_pvc_names(&ship);
        assert_eq!(names, vec!["data".to_string(), "logs".to_string()]);
    }

    #[test]
    fn build_volume_snapshot_sets_owner_and_source_pvc() {
        let snap = build_volume_snapshot(
            "snap-a-data",
            "default",
            "snap-a",
            "snap-a-uid",
            "data",
            Some("fast".to_string()),
        );
        let meta = snap.object_meta.as_ref().unwrap();
        assert_eq!(meta.name.as_deref(), Some("snap-a-data"));
        assert_eq!(meta.namespace.as_deref(), Some("default"));
        let owner = meta.owner_references.first().unwrap();
        assert_eq!(owner.kind, "ShipSnapshot");
        assert_eq!(owner.name, "snap-a");
        assert_eq!(owner.controller, Some(true));
        let source = snap.spec.as_ref().unwrap().source.as_ref().unwrap();
        assert_eq!(source.persistent_volume_claim_name.as_deref(), Some("data"));
        assert!(source.volume_snapshot_content_name.is_none());
        assert_eq!(
            snap.spec
                .as_ref()
                .and_then(|spec| spec.volume_snapshot_class_name.as_deref()),
            Some("fast")
        );
    }

    #[test]
    fn derived_volume_snapshot_name_combines_and_sanitizes() {
        assert_eq!(
            derived_volume_snapshot_name("snap_A", "Data 1"),
            "snap-a-data-1"
        );
        let very_long = "x".repeat(300);
        let name = derived_volume_snapshot_name(&very_long, "pvc");
        assert!(name.len() <= 253);
    }

    #[test]
    fn status_volume_refs_changed_detects_diff() {
        let a = vec![ShipSnapshotVolumeRef {
            pvc_name: "data".to_string(),
            volume_snapshot_name: "snap-a-data".to_string(),
            ready_to_use: Some(false),
        }];
        let b = vec![ShipSnapshotVolumeRef {
            pvc_name: "data".to_string(),
            volume_snapshot_name: "snap-a-data".to_string(),
            ready_to_use: Some(true),
        }];
        let status = ShipSnapshotStatus {
            volume_snapshots: a.clone(),
            ..Default::default()
        };
        assert!(!status_volume_refs_changed(Some(&status), &a));
        assert!(status_volume_refs_changed(Some(&status), &b));
        assert!(status_volume_refs_changed(None, &a));
    }

    fn _shape_check(_: &ShipSnapshotSpec) {}
}
