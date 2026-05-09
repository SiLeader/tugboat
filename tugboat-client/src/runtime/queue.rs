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

use crate::api::Api;
use crate::runtime::runner::process_reconcile_result;
use crate::runtime::{BackoffConfig, ReconcileEvent, Reconciler};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tugboat_resources::{ObjectMetaResource, StaticResource};

#[derive(Clone)]
pub(super) struct ReconcileQueue<T> {
    in_flight: Arc<Mutex<HashMap<String, (u64, CancellationToken)>>>,
    counter: Arc<AtomicU64>,
    _phantom: PhantomData<T>,
}

impl<T> ReconcileQueue<T>
where
    T: StaticResource
        + ObjectMetaResource
        + Serialize
        + DeserializeOwned
        + Clone
        + Send
        + Sync
        + 'static,
{
    pub(super) fn new() -> Self {
        Self {
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            counter: Arc::new(AtomicU64::new(0)),
            _phantom: PhantomData,
        }
    }

    pub(super) fn spawn<R>(
        &self,
        api: Api<T>,
        parent_token: CancellationToken,
        backoff: BackoffConfig,
        event: ReconcileEvent<T>,
        reconciler: R,
    ) where
        R: Reconciler<T>,
    {
        let in_flight = self.in_flight.clone();
        let counter = self.counter.clone();

        let resource_key = match event.resource_key() {
            Some(key) => key,
            None => {
                spawn_uncounted(api, parent_token, backoff, event, reconciler);
                return;
            }
        };

        let id = counter.fetch_add(1, Ordering::Relaxed);

        tokio::spawn(async move {
            let child_token = parent_token.child_token();
            {
                let mut map = in_flight.lock().await;
                if let Some((_, prev)) = map.get(&resource_key) {
                    prev.cancel();
                }
                map.insert(resource_key.clone(), (id, child_token.clone()));
            }

            if child_token.is_cancelled() {
                return;
            }

            let result = tokio::select! {
                _ = child_token.cancelled() => None,
                res = reconciler.reconcile(event.clone()) => Some(res),
            };

            if let Some(res) = result {
                process_reconcile_result(api, child_token, reconciler, &event, res, &backoff, 0)
                    .await;
            }

            let mut map = in_flight.lock().await;
            if let Some((current_id, _)) = map.get(&resource_key)
                && *current_id == id
            {
                map.remove(&resource_key);
            }
        });
    }

    #[cfg(test)]
    pub(super) async fn in_flight_len(&self) -> usize {
        self.in_flight.lock().await.len()
    }
}

fn spawn_uncounted<T, R>(
    api: Api<T>,
    parent_token: CancellationToken,
    backoff: BackoffConfig,
    event: ReconcileEvent<T>,
    reconciler: R,
) where
    T: StaticResource
        + ObjectMetaResource
        + Serialize
        + DeserializeOwned
        + Clone
        + Send
        + Sync
        + 'static,
    R: Reconciler<T>,
{
    let child_token = parent_token.child_token();
    tokio::spawn(async move {
        let result = tokio::select! {
            _ = child_token.cancelled() => None,
            res = reconciler.reconcile(event.clone()) => Some(res),
        };

        if let Some(res) = result {
            process_reconcile_result(api, child_token, reconciler, &event, res, &backoff, 0).await;
        }
    });
}

pub(super) trait ReconcileEventExt<T> {
    fn resource_name(&self) -> Option<&str>;
    fn resource_namespace(&self) -> Option<&str>;
    fn resource_key(&self) -> Option<String>;
}

impl<T> ReconcileEventExt<T> for ReconcileEvent<T>
where
    T: ObjectMetaResource,
{
    fn resource_name(&self) -> Option<&str> {
        match self {
            ReconcileEvent::Applied(resource) | ReconcileEvent::Deleted(resource) => {
                resource.name()
            }
        }
    }

    fn resource_namespace(&self) -> Option<&str> {
        match self {
            ReconcileEvent::Applied(resource) | ReconcileEvent::Deleted(resource) => {
                resource.namespace()
            }
        }
    }

    fn resource_key(&self) -> Option<String> {
        let resource = match self {
            ReconcileEvent::Applied(r) | ReconcileEvent::Deleted(r) => r,
        };
        let name = resource.name()?;
        match resource.namespace() {
            Some(ns) => Some(format!("{ns}/{name}")),
            None => Some(name.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ReconcileEventExt;
    use crate::runtime::ReconcileEvent;
    use tugboat_resources::manifests::core::v1::Ship;
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn event_key_uses_namespace_when_available() {
        let event = ReconcileEvent::Applied(Ship {
            object_meta: Some(ObjectMeta {
                name: Some("demo".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        });

        assert_eq!(event.resource_key().as_deref(), Some("default/demo"));
    }

    #[test]
    fn event_key_is_none_when_name_is_missing() {
        let event = ReconcileEvent::Applied(Ship::default());

        assert_eq!(event.resource_key(), None);
    }
}
