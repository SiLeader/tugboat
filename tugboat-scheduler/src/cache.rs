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

use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::{
    ClusterNetworkClass, NetworkClass, Node, PersistentVolume, PersistentVolumeClaim, Ship,
    ShipClass,
};

pub(crate) struct Cache {
    client: TugboatClient,
    cluster_network_classes: Vec<ClusterNetworkClass>,
    network_classes: Vec<NetworkClass>,
    nodes: Vec<Node>,
    ships: Vec<Ship>,
    ship_classes: Vec<ShipClass>,
    persistent_volume_claims: Vec<PersistentVolumeClaim>,
    persistent_volumes: Vec<PersistentVolume>,
}

impl Cache {
    pub fn new(client: TugboatClient) -> Self {
        Self {
            client,
            cluster_network_classes: Vec::new(),
            network_classes: Vec::new(),
            nodes: Vec::new(),
            ships: Vec::new(),
            ship_classes: Vec::new(),
            persistent_volume_claims: Vec::new(),
            persistent_volumes: Vec::new(),
        }
    }

    pub async fn refresh(&mut self) -> Result<(), tugboat_client::Error> {
        let cluster_network_class_api: Api<ClusterNetworkClass> = Api::all(self.client.clone());
        let network_class_api: Api<NetworkClass> = Api::all(self.client.clone());
        let node_api: Api<Node> = Api::all(self.client.clone());
        let ship_api: Api<Ship> = Api::all(self.client.clone());
        let ship_class_api: Api<ShipClass> = Api::all(self.client.clone());
        let pvc_api: Api<PersistentVolumeClaim> = Api::all(self.client.clone());
        let pv_api: Api<PersistentVolume> = Api::all(self.client.clone());

        self.cluster_network_classes = cluster_network_class_api.list().await?;
        self.network_classes = network_class_api.list().await?;
        self.nodes = node_api.list().await?;
        self.ships = ship_api.list().await?;
        self.ship_classes = ship_class_api.list().await?;
        self.persistent_volume_claims = pvc_api.list().await?;
        self.persistent_volumes = pv_api.list().await?;

        tracing::debug!(
            "Cache refreshed: {} cluster network classes, {} network classes, {} nodes, {} ships, \
             {} ship classes, {} pvcs, {} pvs",
            self.cluster_network_classes.len(),
            self.network_classes.len(),
            self.nodes.len(),
            self.ships.len(),
            self.ship_classes.len(),
            self.persistent_volume_claims.len(),
            self.persistent_volumes.len(),
        );

        Ok(())
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn cluster_network_classes(&self) -> &[ClusterNetworkClass] {
        &self.cluster_network_classes
    }

    pub fn network_classes(&self) -> &[NetworkClass] {
        &self.network_classes
    }

    pub fn ships(&self) -> &[Ship] {
        &self.ships
    }

    pub fn ship_classes(&self) -> &[ShipClass] {
        &self.ship_classes
    }

    pub fn find_ship_class(&self, name: &str) -> Option<&ShipClass> {
        self.ship_classes
            .iter()
            .find(|sc| sc.object_meta.as_ref().and_then(|m| m.name.as_deref()) == Some(name))
    }

    pub fn persistent_volume_claims(&self) -> &[PersistentVolumeClaim] {
        &self.persistent_volume_claims
    }

    pub fn persistent_volumes(&self) -> &[PersistentVolume] {
        &self.persistent_volumes
    }
}
