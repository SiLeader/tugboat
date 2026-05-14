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

use crate::framework::{FilterPlugin, FilterResult, SchedulingContext};
use crate::plugins::selectors::topology_selector_terms_match;
use tugboat_resources::manifests::core::v1::{Node, PersistentVolumeClaim};

const SELECTED_NODE_ANNOTATION: &str = "volume.tugboat.cloud/selected-node";
const WAIT_FOR_FIRST_CONSUMER: &str = "WaitForFirstConsumer";

pub struct StorageBindingReadyFilter;

impl FilterPlugin for StorageBindingReadyFilter {
    fn name(&self) -> &str {
        "StorageBindingReady"
    }

    fn filter(&self, ctx: &SchedulingContext, node: &Node) -> FilterResult {
        let node_name = node
            .object_meta
            .as_ref()
            .and_then(|meta| meta.name.as_deref())
            .unwrap_or("");
        let labels = node
            .object_meta
            .as_ref()
            .map(|meta| &meta.labels)
            .cloned()
            .unwrap_or_default();

        for pvc in ctx.ship_persistent_volume_claims() {
            let Some(spec) = pvc.spec.as_ref() else {
                continue;
            };
            if spec
                .volume_name
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            {
                continue;
            }
            if let Some(selected_node) = selected_node(pvc)
                && selected_node != node_name
            {
                let pvc_name = pvc_name(pvc);
                return FilterResult::Reject(format!(
                    "PersistentVolumeClaim '{pvc_name}' is already selected for node '{selected_node}'"
                ));
            }
            let Some(storage_class_name) = spec
                .storage_class_name
                .as_deref()
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(storage_class_spec) = ctx
                .find_storage_class(storage_class_name)
                .and_then(|storage_class| storage_class.spec.as_ref())
            else {
                continue;
            };
            if storage_class_spec.volume_binding_mode.as_deref() != Some(WAIT_FOR_FIRST_CONSUMER) {
                continue;
            }
            if !topology_selector_terms_match(&storage_class_spec.allowed_topologies, &labels) {
                let pvc_name = pvc_name(pvc);
                return FilterResult::Reject(format!(
                    "PersistentVolumeClaim '{pvc_name}' StorageClass '{storage_class_name}' allowedTopologies do not match the node"
                ));
            }
        }

        FilterResult::Accept
    }
}

fn selected_node(pvc: &PersistentVolumeClaim) -> Option<&str> {
    pvc.object_meta
        .as_ref()
        .and_then(|meta| meta.annotations.get(SELECTED_NODE_ANNOTATION))
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

fn pvc_name(pvc: &PersistentVolumeClaim) -> &str {
    pvc.object_meta
        .as_ref()
        .and_then(|meta| meta.name.as_deref())
        .unwrap_or("<unknown>")
}
