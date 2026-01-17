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

use crate::cni::NetworkClassInfo;
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::ReconcileError;
use tugboat_client::Api;
use tugboat_resources::manifests::core::v1::{
    ClusterNetworkClass, NetworkClass, ShipNetworkClassRef, ShipSpec,
};

impl ShipReconciler {
    pub(crate) async fn get_related_network_classes(
        &self,
        namespace: &str,
        ship_spec: &ShipSpec,
    ) -> Result<Vec<NetworkClassInfo>, ReconcileError> {
        let cluster_scoped_api: Api<ClusterNetworkClass> = Api::all(self.client.clone());
        let namespaced_api: Api<NetworkClass> = Api::namespaced(self.client.clone(), namespace);

        let mut ncs = Vec::new();
        for nc in &ship_spec.network_class_ref {
            let spec = self
                .get_network_class_spec(&cluster_scoped_api, &namespaced_api, namespace, nc)
                .await?;
            ncs.push(spec);
        }

        Ok(ncs)
    }

    async fn get_network_class_spec(
        &self,
        cluster_scoped_api: &Api<ClusterNetworkClass>,
        namespaced_api: &Api<NetworkClass>,
        namespace: &str,
        nc_ref: &ShipNetworkClassRef,
    ) -> Result<NetworkClassInfo, ReconcileError> {
        if nc_ref.api_group != "core" && !nc_ref.api_group.is_empty() {
            return Err(ReconcileError::InvalidNetworkClassRef(
                nc_ref.clone().into(),
            ));
        }

        let spec = if nc_ref.kind == "ClusterNetworkClass" {
            match cluster_scoped_api.get(&nc_ref.name).await? {
                Some(n) => n.spec,
                None => return Err(ReconcileError::NetworkClassNotFound(nc_ref.clone().into())),
            }
        } else if nc_ref.kind == "NetworkClass" {
            match namespaced_api.get(&nc_ref.name).await? {
                Some(n) => n.spec,
                None => return Err(ReconcileError::NetworkClassNotFound(nc_ref.clone().into())),
            }
        } else {
            return Err(ReconcileError::InvalidNetworkClassRef(
                nc_ref.clone().into(),
            ));
        };
        let spec =
            spec.ok_or_else(|| ReconcileError::InvalidNetworkClassRef(nc_ref.clone().into()))?;

        Ok(NetworkClassInfo {
            name: nc_ref.name.clone(),
            namespace: if nc_ref.kind == "ClusterNetworkClass" {
                None
            } else {
                Some(namespace.to_string())
            },
            spec,
        })
    }
}
