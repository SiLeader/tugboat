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

use crate::csi::PublishedVolume;
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use crate::reconciler::ops::add_helpers::{
    apply_persistent_volume_claim_csi_observation, apply_persistent_volume_csi_observation,
};
use crate::reconciler::volume::PersistentVolumeClaimVolumeInfo;
use tugboat_client::Api;
use tugboat_resources::manifests::core::v1::{
    PersistentVolume, PersistentVolumeClaim, PersistentVolumeClaimStatus,
};

impl ShipReconciler {
    pub(super) async fn ensure_node_expansion(
        &self,
        namespace: &str,
        volume: &PersistentVolumeClaimVolumeInfo,
        published: &PublishedVolume,
        secrets: &crate::csi::ResolvedCsiSecrets,
    ) -> Result<(), ReconcileError> {
        let needs_node_expansion = volume
            .status
            .as_ref()
            .and_then(|status| status.node_expansion_required)
            .unwrap_or(false);
        if !needs_node_expansion {
            return Ok(());
        }
        let Some(target_capacity_bytes) = volume
            .status
            .as_ref()
            .and_then(|status| status.capacity_bytes)
            .or(volume.volume.capacity_bytes)
            .or(volume.claim.requested_capacity_bytes)
            .filter(|value| *value > 0)
        else {
            return Ok(());
        };
        let expanded_capacity_bytes = self
            .csi
            .expand(
                published,
                &volume.source,
                &volume.claim,
                &volume.volume,
                secrets,
                target_capacity_bytes,
            )
            .await?;
        let Some(expanded_capacity_bytes) = expanded_capacity_bytes else {
            return Err(ReconcileError::UnsupportedPersistentVolumeCsiFeature {
                volume: volume.volume_name.clone(),
                feature: "node_expand".to_string(),
            });
        };
        self.mark_volume_node_expanded(
            namespace,
            &volume.claim_name,
            &volume.volume_name,
            expanded_capacity_bytes,
        )
        .await?;
        Ok(())
    }

    pub(super) async fn refresh_volume_stats(
        &self,
        namespace: &str,
        volume: &PersistentVolumeClaimVolumeInfo,
        published: &PublishedVolume,
    ) -> Result<(), ReconcileError> {
        let Some(stats) = self.csi.volume_stats(published).await? else {
            return Ok(());
        };

        let pv_api: Api<PersistentVolume> = Api::all(self.client.clone());
        let Some(mut persistent_volume) = pv_api.get(&volume.volume_name).await? else {
            return Ok(());
        };
        let mut persistent_volume_changed = false;
        {
            let status = persistent_volume
                .status
                .get_or_insert_with(Default::default);
            persistent_volume_changed |=
                apply_persistent_volume_csi_observation(&mut status.conditions, &stats);
        }
        if persistent_volume_changed {
            pv_api
                .replace(&volume.volume_name, persistent_volume)
                .await?;
        }

        let pvc_api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);
        let Some(mut claim) = pvc_api.get(&volume.claim_name).await? else {
            return Ok(());
        };
        let mut claim_changed = false;
        {
            let status = claim
                .status
                .get_or_insert_with(PersistentVolumeClaimStatus::default);
            claim_changed |=
                apply_persistent_volume_claim_csi_observation(&mut status.conditions, &stats);
        }
        if claim_changed {
            pvc_api.replace(&volume.claim_name, claim).await?;
        }

        Ok(())
    }

    pub(super) async fn mark_volume_attached(
        &self,
        volume_name: &str,
        attached: bool,
    ) -> Result<(), ReconcileError> {
        let api: Api<PersistentVolume> = Api::all(self.client.clone());
        let Some(mut volume) = api.get(volume_name).await? else {
            return Ok(());
        };
        let status = volume.status.get_or_insert_with(Default::default);
        status.attached_node = attached.then(|| self.node_name.clone());
        if status.phase.is_none() {
            status.phase = Some("Bound".to_string());
        }
        api.replace(volume_name, volume).await?;
        Ok(())
    }

    async fn mark_volume_node_expanded(
        &self,
        namespace: &str,
        claim_name: &str,
        volume_name: &str,
        capacity_bytes: i64,
    ) -> Result<(), ReconcileError> {
        let api: Api<PersistentVolume> = Api::all(self.client.clone());
        let Some(mut volume) = api.get(volume_name).await? else {
            return Ok(());
        };
        if let Some(spec) = volume.spec.as_mut() {
            spec.capacity_bytes = Some(capacity_bytes);
        }
        let status = volume.status.get_or_insert_with(Default::default);
        status.phase = Some("Bound".to_string());
        status.capacity_bytes = Some(capacity_bytes);
        status.node_expansion_required = Some(false);
        status.attached_node = Some(self.node_name.clone());
        api.replace(volume_name, volume).await?;
        self.mark_claim_node_expanded(namespace, claim_name, capacity_bytes)
            .await?;
        Ok(())
    }

    async fn mark_claim_node_expanded(
        &self,
        namespace: &str,
        claim_name: &str,
        capacity_bytes: i64,
    ) -> Result<(), ReconcileError> {
        let api: Api<PersistentVolumeClaim> = Api::namespaced(self.client.clone(), namespace);
        let Some(mut claim) = api.get(claim_name).await? else {
            return Ok(());
        };
        let status = claim
            .status
            .get_or_insert_with(PersistentVolumeClaimStatus::default);
        let mut changed = false;
        if status.phase.as_deref() != Some("Bound") {
            status.phase = Some("Bound".to_string());
            changed = true;
        }
        if status.capacity_bytes != Some(capacity_bytes) {
            status.capacity_bytes = Some(capacity_bytes);
            changed = true;
        }
        if status.resize_pending != Some(false) {
            status.resize_pending = Some(false);
            changed = true;
        }
        if changed {
            api.replace(claim_name, claim).await?;
        }
        Ok(())
    }
}
