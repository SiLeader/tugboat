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
use tugboat_resources::manifests::core::v1::{Node, Ship, ShipClass};

pub(crate) struct Cache {
    client: TugboatClient,
    nodes: Vec<Node>,
    ships: Vec<Ship>,
    ship_classes: Vec<ShipClass>,
}

impl Cache {
    pub fn new(client: TugboatClient) -> Self {
        Self {
            client,
            nodes: Vec::new(),
            ships: Vec::new(),
            ship_classes: Vec::new(),
        }
    }

    pub async fn refresh(&mut self) -> Result<(), tugboat_client::Error> {
        let node_api: Api<Node> = Api::all(self.client.clone());
        let ship_api: Api<Ship> = Api::all(self.client.clone());
        let ship_class_api: Api<ShipClass> = Api::all(self.client.clone());

        self.nodes = node_api.list().await?;
        self.ships = ship_api.list().await?;
        self.ship_classes = ship_class_api.list().await?;

        tracing::debug!(
            "Cache refreshed: {} nodes, {} ships, {} ship classes",
            self.nodes.len(),
            self.ships.len(),
            self.ship_classes.len()
        );

        Ok(())
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
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
}
