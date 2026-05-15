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
use crate::config::ControllerManagerConfig;
use crate::error::ControllerError;
use crate::provisioning::storage_class_provisioner;
use std::collections::BTreeSet;
use tugboat_client::runtime::{
    Action, Controller, FinalizerEvent, ReconcileEvent, Reconciler, finalizer,
};
use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::{
    PersistentVolumeClaim, Ship, ShipSnapshot, ShipSnapshotStatus, ShipSnapshotVolumeRef,
    StorageClass,
};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, OwnerReference};
use tugboat_resources::manifests::snapshot::v1::{
    VolumeSnapshot, VolumeSnapshotClass, VolumeSnapshotSource, VolumeSnapshotSpec,
};
use tugboat_resources::{ObjectMetaResource, Resource};

const SHIP_SNAPSHOT_VOLUMES_FINALIZER: &str = "snapshot.tugboat.cloud/volume-fanout";
const DEFAULT_VOLUME_SNAPSHOT_CLASS_ANNOTATION: &str =
    "snapshot.storage.kubernetes.io/is-default-class";

#[derive(Clone)]
struct ShipSnapshotVolumesReconciler {
    client: TugboatClient,
    config: ControllerManagerConfig,
}

pub(crate) struct ShipSnapshotVolumesController {
    controller: Controller<ShipSnapshot>,
    reconciler: ShipSnapshotVolumesReconciler,
}

impl ShipSnapshotVolumesController {
    pub(crate) fn new(client: TugboatClient, config: ControllerManagerConfig) -> Self {
        Self {
            controller: Controller::new(Api::all(client.clone())),
            reconciler: ShipSnapshotVolumesReconciler { client, config },
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
            ReconcileEvent::Deleted(snapshot) => self.cleanup_volume_snapshots(snapshot).await,
        }
    }
}

impl ShipSnapshotVolumesReconciler {
    fn requeue_action(&self) -> Action {
        Action::requeue(self.config.csi.requeue_interval())
    }

    async fn reconcile_applied(&self, snapshot: ShipSnapshot) -> Result<Action, ControllerError> {
        if !ship_snapshot_includes_volumes(&snapshot) && snapshot.deletion_timestamp().is_none() {
            return Ok(Action::await_change());
        }
        let namespace = snapshot
            .namespace()
            .ok_or(ControllerError::MissingNamespace("ShipSnapshot"))?
            .to_string();
        let api: Api<ShipSnapshot> = Api::namespaced(self.client.clone(), &namespace);
        finalizer(&api, SHIP_SNAPSHOT_VOLUMES_FINALIZER, snapshot, {
            let this = self.clone();
            move |event| async move {
                match event {
                    FinalizerEvent::Apply(snapshot) => {
                        this.reconcile_active_snapshot(snapshot).await
                    }
                    FinalizerEvent::Cleanup(snapshot) => {
                        this.cleanup_volume_snapshots(snapshot).await
                    }
                }
            }
        })
        .await
        .map_err(|err| ControllerError::Finalizer(err.to_string()))
    }

    async fn reconcile_active_snapshot(
        &self,
        snapshot: ShipSnapshot,
    ) -> Result<Action, ControllerError> {
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
            let snapshot_class_name = self
                .resolve_volume_snapshot_class_name(
                    &namespace,
                    pvc_name,
                    spec.volume_snapshot_class_name.as_deref(),
                )
                .await?;
            let Some(snapshot_class_name) = snapshot_class_name else {
                refs.push(ShipSnapshotVolumeRef {
                    pvc_name: pvc_name.clone(),
                    volume_snapshot_name,
                    ready_to_use: None,
                });
                continue;
            };
            let desired = build_volume_snapshot(
                &volume_snapshot_name,
                &namespace,
                &name,
                &uid,
                pvc_name,
                Some(snapshot_class_name),
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
            return Ok(self.requeue_action());
        }
        if refs.iter().any(|vol| vol.ready_to_use != Some(true)) {
            return Ok(self.requeue_action());
        }
        Ok(Action::await_change())
    }

    async fn resolve_volume_snapshot_class_name(
        &self,
        namespace: &str,
        pvc_name: &str,
        requested_class_name: Option<&str>,
    ) -> Result<Option<String>, ControllerError> {
        if let Some(class_name) = requested_class_name
            .map(str::trim)
            .filter(|class_name| !class_name.is_empty())
        {
            return Ok(Some(class_name.to_string()));
        }

        let driver = self
            .storage_driver_for_pvc(namespace, pvc_name)
            .await?
            .map(|driver| driver.to_string());
        let class_api: Api<VolumeSnapshotClass> = Api::all(self.client.clone());
        let classes = class_api.list().await?;
        Ok(default_volume_snapshot_class_name(
            classes.as_slice(),
            driver.as_deref(),
        ))
    }

    async fn storage_driver_for_pvc(
        &self,
        namespace: &str,
        pvc_name: &str,
    ) -> Result<Option<String>, ControllerError> {
        let pvc_api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);
        let Some(pvc) = pvc_api.get(pvc_name).await? else {
            return Ok(None);
        };
        let Some(storage_class_name) = pvc
            .spec
            .as_ref()
            .and_then(|spec| spec.storage_class_name.as_deref())
            .filter(|name| !name.is_empty())
        else {
            return Ok(None);
        };
        let storage_class_api: Api<StorageClass> = Api::all(self.client.clone());
        let Some(storage_class) = storage_class_api.get(storage_class_name).await? else {
            return Ok(None);
        };
        let (driver, _) = storage_class_provisioner(&storage_class)?;
        Ok(Some(driver))
    }

    async fn cleanup_volume_snapshots(
        &self,
        snapshot: ShipSnapshot,
    ) -> Result<Action, ControllerError> {
        let namespace = snapshot
            .namespace()
            .ok_or(ControllerError::MissingNamespace("ShipSnapshot"))?
            .to_string();
        let snapshot_api: Api<VolumeSnapshot> = Api::namespaced(self.client.clone(), &namespace);
        let names = self.volume_snapshot_names_for_cleanup(&snapshot).await?;
        for volume_snapshot_name in names {
            match snapshot_api.delete(&volume_snapshot_name).await {
                Ok(_) => {}
                Err(tugboat_client::Error::Api(status)) if status.code == 404 => {}
                Err(err) => return Err(err.into()),
            }
        }
        Ok(Action::await_change())
    }

    async fn volume_snapshot_names_for_cleanup(
        &self,
        snapshot: &ShipSnapshot,
    ) -> Result<BTreeSet<String>, ControllerError> {
        let namespace = snapshot
            .namespace()
            .ok_or(ControllerError::MissingNamespace("ShipSnapshot"))?;
        let name = snapshot
            .name()
            .ok_or(ControllerError::MissingName("ShipSnapshot"))?;
        let uid = snapshot
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.uid.as_deref());
        let mut names = BTreeSet::new();

        if let Some(status) = snapshot.status.as_ref() {
            names.extend(
                status
                    .volume_snapshots
                    .iter()
                    .filter(|volume| !volume.volume_snapshot_name.is_empty())
                    .map(|volume| volume.volume_snapshot_name.clone()),
            );
        }

        if let Some(spec) = snapshot.spec.as_ref()
            && spec.include_volumes.unwrap_or(false)
        {
            let ship_api: Api<Ship> = Api::namespaced(self.client.clone(), namespace);
            if let Some(ship) = ship_api.get(&spec.ship_name).await? {
                names.extend(
                    ship_pvc_names(&ship)
                        .iter()
                        .map(|pvc_name| derived_volume_snapshot_name(name, pvc_name)),
                );
            }
        }

        let snapshot_api: Api<VolumeSnapshot> = Api::namespaced(self.client.clone(), namespace);
        for volume_snapshot in snapshot_api.list().await? {
            if volume_snapshot_owned_by(&volume_snapshot, name, uid)
                && let Some(volume_snapshot_name) = volume_snapshot.name()
            {
                names.insert(volume_snapshot_name.to_string());
            }
        }

        Ok(names)
    }
}

fn ship_snapshot_includes_volumes(snapshot: &ShipSnapshot) -> bool {
    snapshot
        .spec
        .as_ref()
        .and_then(|spec| spec.include_volumes)
        .unwrap_or(false)
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

fn default_volume_snapshot_class_name(
    classes: &[VolumeSnapshotClass],
    driver: Option<&str>,
) -> Option<String> {
    classes
        .iter()
        .filter(|class| is_default_volume_snapshot_class(class))
        .filter(|class| {
            driver.is_none_or(|driver| {
                class
                    .spec
                    .as_ref()
                    .is_some_and(|spec| spec.driver == driver)
            })
        })
        .find_map(|class| class.name().map(ToOwned::to_owned))
}

fn is_default_volume_snapshot_class(class: &VolumeSnapshotClass) -> bool {
    class
        .object_meta()
        .as_ref()
        .and_then(|meta| {
            meta.annotations
                .get(DEFAULT_VOLUME_SNAPSHOT_CLASS_ANNOTATION)
        })
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn volume_snapshot_owned_by(
    volume_snapshot: &VolumeSnapshot,
    owner_name: &str,
    owner_uid: Option<&str>,
) -> bool {
    volume_snapshot.object_meta().as_ref().is_some_and(|meta| {
        meta.owner_references.iter().any(|reference| {
            reference.kind == "ShipSnapshot"
                && reference.name == owner_name
                && owner_uid.is_none_or(|uid| reference.uid == uid)
        })
    })
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
    use std::collections::HashMap;
    use tugboat_resources::manifests::core::v1::{
        PersistentVolumeClaimVolumeSource, ShipSnapshotSpec, ShipSpec, ShipVolume,
        ShipVolumeClaimReference,
    };
    use tugboat_resources::manifests::meta::v1::ObjectMeta;
    use tugboat_resources::manifests::snapshot::v1::VolumeSnapshotClassSpec;

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

    #[test]
    fn default_volume_snapshot_class_filters_by_annotation_and_driver() {
        let classes = vec![
            snapshot_class("slow", "csi.slow", true),
            snapshot_class("fast", "csi.fast", true),
            snapshot_class("unmarked", "csi.fast", false),
        ];

        assert_eq!(
            default_volume_snapshot_class_name(&classes, Some("csi.fast")).as_deref(),
            Some("fast")
        );
        assert_eq!(
            default_volume_snapshot_class_name(&classes, Some("csi.missing")),
            None
        );
    }

    #[test]
    fn volume_snapshot_owned_by_matches_ship_snapshot_owner() {
        let snap = build_volume_snapshot(
            "snap-a-data",
            "default",
            "snap-a",
            "snap-a-uid",
            "data",
            Some("fast".to_string()),
        );

        assert!(volume_snapshot_owned_by(
            &snap,
            "snap-a",
            Some("snap-a-uid")
        ));
        assert!(!volume_snapshot_owned_by(
            &snap,
            "snap-a",
            Some("other-uid")
        ));
        assert!(!volume_snapshot_owned_by(&snap, "other", None));
    }

    fn snapshot_class(name: &str, driver: &str, is_default: bool) -> VolumeSnapshotClass {
        let mut annotations = HashMap::new();
        if is_default {
            annotations.insert(
                DEFAULT_VOLUME_SNAPSHOT_CLASS_ANNOTATION.to_string(),
                "true".to_string(),
            );
        }
        VolumeSnapshotClass {
            object_meta: Some(ObjectMeta {
                name: Some(name.to_string()),
                annotations,
                ..Default::default()
            }),
            spec: Some(VolumeSnapshotClassSpec {
                driver: driver.to_string(),
                deletion_policy: "Delete".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn _shape_check(_: &ShipSnapshotSpec) {}
}
