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
use tokio::sync::RwLock;
use tugboat_vm_image::VmImageRegistry;
use tugboat_vm_runtime_interface::operator::VmRuntimeOperator;

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
        fingerprints: crate::reconciler::ShipFingerprints,
        published_volumes: Vec<crate::csi::PublishedVolume>,
    ) {
        let mut children = self.children.write().await;
        children.insert(
            id.clone(),
            inner::Runtime::new(namespace, ship_name, id, fingerprints, published_volumes),
        );
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RuntimeConfig {
    pub executable: String,
    pub args: Vec<String>,
}
