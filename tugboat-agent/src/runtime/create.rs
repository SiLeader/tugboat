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
use crate::reconciler::ShipFingerprints;
use crate::runtime::RuntimeOperator;
use crate::runtime::error::RuntimeError;
use crate::runtime::inner::{Runtime, RuntimeSpecState};
use tracing::{debug, info};
use tugboat_resources::manifests::core::v1::{ShipClass, ShipSpec};
use tugboat_resources::sized::SizedString;
use tugboat_vm_runtime_interface::run::{
    VmCpuConfig, VmMemoryConfig, VmNetworkConfig, VmRunRequest, VmUefiConfig, VmVolumeConfig,
};

pub(crate) struct RuntimeCreateRequest<'a> {
    pub ship_id: String,
    pub ship_name: String,
    pub namespace: String,
    pub ship_spec: &'a ShipSpec,
    pub ship_class: ShipClass,
    pub incoming_port: Option<u16>,
    pub networks: Vec<VmNetworkConfig>,
    pub volumes: Vec<VmVolumeConfig>,
    pub fingerprints: ShipFingerprints,
    pub published_volumes: Vec<PublishedVolume>,
}

impl RuntimeOperator {
    fn is_http_host(&self, image: &str) -> Option<bool> {
        let (host, _) = image.split_once('/')?;
        Some(self.http_hosts.contains(host))
    }

    pub(crate) async fn create(
        &self,
        request: RuntimeCreateRequest<'_>,
    ) -> Result<u32, RuntimeError> {
        let RuntimeCreateRequest {
            ship_id,
            ship_name,
            namespace,
            ship_spec,
            ship_class,
            incoming_port,
            networks,
            volumes,
            fingerprints,
            published_volumes,
        } = request;
        let Some(ship_class_spec) = ship_class.spec else {
            return Err(RuntimeError::MissingField("v1.ShipClass.spec".to_string()));
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
        debug!("Pulling image '{}'", ship_spec.image);
        let image = self
            .registry
            .pull(&ship_spec.image, self.is_http_host(&ship_spec.image))
            .await?;

        let memory_size = SizedString(memory.size.clone());
        let memory_size = memory_size
            .as_byte_length()
            .ok_or(RuntimeError::MemorySize(memory.size.clone()))?;
        let memory_max_size = memory
            .max_size
            .clone()
            .map(SizedString)
            .map(|size| {
                size.as_byte_length()
                    .ok_or(RuntimeError::MemorySize(size.0))
            })
            .transpose()?;

        let vm_config = VmRunRequest {
            id: ship_id.clone(),
            image: image.location,
            cpu: VmCpuConfig {
                architecture: cpu.architecture,
                cores: cpu.cores,
                max_cores: cpu.max_cores,
            },
            memory: VmMemoryConfig {
                size: memory_size,
                max_size: memory_max_size,
                slots: memory.slots,
            },
            networks,
            incoming: incoming_port
                .map(|port| tugboat_vm_runtime_interface::run::VmIncomingMigrationConfig { port }),
            user: Default::default(),
            uefi: VmUefiConfig {
                enabled: ship_spec.uefi.map(|u| u.enabled).unwrap_or(false),
            },
            volumes,
        };
        debug!("Creating VM: {:?}", vm_config);
        let pid = self.operator.create(vm_config).await?;
        info!("Create VM '{ship_id}' called successfully",);
        let mut children = self.children.write().await;
        children.insert(
            ship_id.clone(),
            Runtime::new(
                namespace,
                ship_name,
                ship_id,
                fingerprints,
                published_volumes,
                RuntimeSpecState {
                    image: ship_spec.image.clone(),
                    network_class_ref: ship_spec.network_class_ref.clone(),
                    uefi: ship_spec.uefi.clone(),
                    cpu_cores: cpu.cores,
                    memory_size,
                },
            ),
        );
        Ok(pid)
    }
}
