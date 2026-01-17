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

use crate::runtime::error::RuntimeError;
use crate::runtime::inner::Runtime;
use crate::runtime::{RuntimeOperator, handle_command_response};
use tugboat_resources::manifests::core::v1::{Ship, ShipClass};
use tugboat_resources::sized::SizedString;
use tugboat_vm_runtime_interface::start::{VmCpuConfig, VmNetworkConfig, VmStartRequest};

impl RuntimeOperator {
    pub(crate) async fn start(
        &self,
        ship: Ship,
        ship_class: ShipClass,
        networks: Vec<VmNetworkConfig>,
    ) -> Result<(), RuntimeError> {
        let Some(object_meta) = ship.object_meta else {
            return Err(RuntimeError::MissingField("v1.Ship.metadata".to_string()));
        };
        let Some(ship_id) = object_meta.uid else {
            return Err(RuntimeError::MissingField(
                "v1.Ship.metadata.uid".to_string(),
            ));
        };
        let namespace = object_meta.namespace.unwrap_or("default".to_string());
        let Some(ship_class_spec) = ship_class.spec else {
            return Err(RuntimeError::MissingField("v1.ShipClass.spec".to_string()));
        };
        let Some(ship_spec) = ship.spec else {
            return Err(RuntimeError::MissingField("v1.Ship.spec".to_string()));
        };
        let Some(cpu) = ship_class_spec.cpu else {
            return Err(RuntimeError::MissingField(
                "v1.ShipClass.spec.cpu".to_string(),
            ));
        };
        let Some(memory) = ship_class_spec.memory else {
            return Err(RuntimeError::MissingField(
                "v1.ShipClass.spec.memory".to_string(),
            ));
        };
        let image = self.registry.pull(ship_spec.image, None).await?;

        let memory_size = SizedString(memory.size.clone());

        let vm_config = VmStartRequest {
            id: ship_id.clone(),
            image: image.location,
            cpu: VmCpuConfig {
                architecture: cpu.architecture,
                cores: cpu.cores,
            },
            memory: memory_size
                .as_byte_length()
                .ok_or(RuntimeError::MemorySize(memory.size))?,
            networks,
            user: Default::default(),
        };
        handle_command_response(self.run_command("start", &vm_config).await?).await?;
        let mut children = self.children.write().await;
        children.insert(ship_id.clone(), Runtime::new(namespace, ship_id));
        Ok(())
    }
}
