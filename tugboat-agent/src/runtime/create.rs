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
use crate::runtime::inner::Runtime;
use tracing::{debug, info};
use tugboat_resources::manifests::core::v1::{ShipClass, ShipSpec};
use tugboat_resources::sized::SizedString;
use tugboat_vm_image::Format as VmImageFormat;
use tugboat_vm_runtime_interface::run::{
    VmCpuConfig, VmDiskImageFormat, VmMemoryConfig, VmNetworkConfig, VmRunRequest, VmUefiConfig,
    VmVolumeConfig,
};

pub(crate) struct RuntimeCreateRequest<'a> {
    pub ship_id: String,
    pub ship_name: String,
    pub namespace: String,
    pub ship_spec: &'a ShipSpec,
    pub ship_class: ShipClass,
    pub incoming_port: Option<u16>,
    pub restore_handle: Option<String>,
    pub restore_source_id: Option<String>,
    pub networks: Vec<VmNetworkConfig>,
    pub volumes: Vec<VmVolumeConfig>,
    pub fingerprints: ShipFingerprints,
    pub published_volumes: Vec<PublishedVolume>,
}

impl RuntimeOperator {
    fn is_http_host(&self, image: &str) -> bool {
        if let Some((host, _)) = image.split_once('/') {
            self.http_hosts.contains(host)
        } else {
            false
        }
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
            restore_handle,
            restore_source_id,
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
            .pull(&ship_spec.image, Some(self.is_http_host(&ship_spec.image)))
            .await?;

        let memory_size = SizedString(memory.size.clone());
        let memory_size = memory_size
            .as_byte_length()
            .ok_or(RuntimeError::MemorySize(memory.size.clone()))?;
        let vm_config = VmRunRequest {
            id: ship_id.clone(),
            image: image.location,
            image_format: runtime_image_format(image.format),
            cpu: VmCpuConfig {
                architecture: cpu.architecture,
                cores: cpu.cores,
            },
            memory: VmMemoryConfig { size: memory_size },
            networks,
            incoming: incoming_port
                .map(|port| tugboat_vm_runtime_interface::run::VmIncomingMigrationConfig { port }),
            restore_handle,
            restore_source_id,
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
                ship_spec.clone(),
                fingerprints,
                published_volumes,
            ),
        );
        Ok(pid)
    }
}

fn runtime_image_format(format: VmImageFormat) -> VmDiskImageFormat {
    match format {
        VmImageFormat::Qcow2 => VmDiskImageFormat::Qcow2,
        VmImageFormat::Raw => VmDiskImageFormat::Raw,
    }
}

#[cfg(test)]
mod tests {
    use super::{VmDiskImageFormat, VmImageFormat, runtime_image_format};

    #[test]
    fn runtime_image_format_maps_pulled_image_format() {
        assert_eq!(
            runtime_image_format(VmImageFormat::Qcow2),
            VmDiskImageFormat::Qcow2
        );
        assert_eq!(
            runtime_image_format(VmImageFormat::Raw),
            VmDiskImageFormat::Raw
        );
    }
}
