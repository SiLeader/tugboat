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

mod create;
mod delete;
pub(crate) mod error;
mod hotplug;
mod inner;
mod migration;
mod start;
mod status;

use crate::runtime::inner::Runtime;
pub(crate) use create::RuntimeCreateRequest;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tugboat_resources::manifests::core::v1::ShipSpec;
use tugboat_vm_image::VmImageRegistry;
use tugboat_vm_runtime_interface::operator::VmRuntimeOperator;

use crate::csi::PublishedVolume;
use crate::reconciler::ShipFingerprints;

#[derive(Clone)]
pub(crate) struct RuntimeOperator {
    operator: VmRuntimeOperator,
    registry: VmImageRegistry,
    children: Arc<RwLock<HashMap<String, Runtime>>>,
    http_hosts: HashSet<String>,
}

impl RuntimeOperator {
    pub(crate) fn new(
        config: RuntimeConfig,
        image_dir: impl AsRef<Path>,
        http_hosts: HashSet<String>,
    ) -> Self {
        Self {
            operator: VmRuntimeOperator::new(config.executable, config.args),
            registry: VmImageRegistry::new(image_dir.as_ref()),
            children: Arc::new(RwLock::new(HashMap::new())),
            http_hosts,
        }
    }

    pub(crate) async fn has_ship(&self, id: &str) -> bool {
        self.children.read().await.contains_key(id)
    }

    pub(crate) async fn matches_spec_fingerprint(&self, id: &str, fingerprint: &str) -> bool {
        self.children
            .read()
            .await
            .get(id)
            .is_some_and(|runtime| runtime.matches_spec_fingerprint(fingerprint))
    }

    pub(crate) async fn matches_pvc_volume_fingerprint(&self, id: &str, fingerprint: &str) -> bool {
        self.children
            .read()
            .await
            .get(id)
            .is_some_and(|runtime| runtime.matches_pvc_volume_fingerprint(fingerprint))
    }

    pub(crate) async fn matches_materialized_volume_fingerprint(
        &self,
        id: &str,
        fingerprint: &str,
    ) -> bool {
        self.children
            .read()
            .await
            .get(id)
            .is_some_and(|runtime| runtime.matches_materialized_volume_fingerprint(fingerprint))
    }

    pub(crate) async fn update_materialized_volume_fingerprint(
        &self,
        id: &str,
        fingerprint: String,
    ) {
        if let Some(runtime) = self.children.write().await.get_mut(id) {
            runtime.update_materialized_volume_fingerprint(fingerprint);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn register_existing(
        &self,
        namespace: String,
        ship_name: String,
        id: String,
        ship_spec: ShipSpec,
        fingerprints: ShipFingerprints,
        published_volumes: Vec<PublishedVolume>,
    ) {
        let mut children = self.children.write().await;
        children.insert(
            id.clone(),
            Runtime::new(
                namespace,
                ship_name,
                id,
                ship_spec,
                fingerprints,
                published_volumes,
            ),
        );
    }

    pub(crate) async fn current_ship_spec(&self, id: &str) -> Option<ShipSpec> {
        self.children
            .read()
            .await
            .get(id)
            .map(|runtime| runtime.ship_spec().clone())
    }

    pub(crate) async fn current_published_volumes(&self, id: &str) -> Option<Vec<PublishedVolume>> {
        self.children
            .read()
            .await
            .get(id)
            .map(|runtime| runtime.published_volumes().to_vec())
    }

    pub(crate) async fn update_runtime_state(
        &self,
        id: &str,
        ship_spec: ShipSpec,
        fingerprints: ShipFingerprints,
        published_volumes: Vec<PublishedVolume>,
    ) {
        if let Some(runtime) = self.children.write().await.get_mut(id) {
            runtime.update_runtime_state(ship_spec, fingerprints, published_volumes);
        }
    }

    /// Returns the per-ship hotplug lock, which must be held for the duration of
    /// any hotplug operation to prevent concurrent modifications for the same ship.
    pub(crate) async fn get_hotplug_lock(&self, id: &str) -> Option<Arc<Mutex<()>>> {
        self.children
            .read()
            .await
            .get(id)
            .map(|runtime| runtime.hotplug_lock())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RuntimeConfig {
    pub executable: String,
    pub args: Vec<String>,
}
