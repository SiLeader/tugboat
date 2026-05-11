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

use crate::name_generator::NameGenerator;
use std::sync::Arc;
use tugboat_resource_store::ResourceStore;
use tugboat_resources::manifests::meta::v1::{ObjectMeta, Time};
use uuid::Uuid;

pub(crate) struct ApiOperator {
    pub(crate) store: ResourceStore,
    pub(crate) name_generator: NameGenerator,
    pub(crate) service_account_tokens:
        Option<Arc<crate::auth::service_account_jwt::ServiceAccountTokenIssuer>>,
}

impl ApiOperator {
    pub(crate) fn new(
        store: ResourceStore,
        service_account_tokens: Option<crate::auth::service_account_jwt::ServiceAccountTokenIssuer>,
    ) -> Self {
        Self {
            store,
            name_generator: NameGenerator::new(),
            service_account_tokens: service_account_tokens.map(Arc::new),
        }
    }

    pub(crate) fn apply_namespace(
        &self,
        mut object_meta: ObjectMeta,
        namespace: String,
    ) -> ObjectMeta {
        object_meta.namespace = Some(namespace);
        object_meta
    }

    pub(crate) fn apply_uid(&self, mut object_meta: ObjectMeta) -> ObjectMeta {
        object_meta.uid = Some(Uuid::new_v4().to_string());
        object_meta.creation_timestamp = Some(Time::now());
        object_meta.generation = Some(1);
        object_meta.resource_version = None;
        object_meta
    }
}
