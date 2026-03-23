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
use crate::runtime::{Action, BackoffConfig, ReconcileEvent, Reconciler};
use crate::{Error, WatchEvent, WatchParams};
use futures::{Stream, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::error;
use tugboat_resources::{ObjectMetaResource, StaticResource};

#[derive(Clone)]
pub struct Controller<T> {
    api: Api<T>,
    watch_params: WatchParams,
    backoff: BackoffConfig,
    cancellation_token: CancellationToken,
}

impl<T> Controller<T>
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
    pub fn new(api: Api<T>) -> Self {
        Self {
            api,
            watch_params: WatchParams::default(),
            backoff: BackoffConfig::default(),
            cancellation_token: CancellationToken::new(),
        }
    }

    pub fn with_watch_params(mut self, watch_params: WatchParams) -> Self {
        self.watch_params = watch_params;
        self
    }

    pub fn with_backoff(mut self, backoff: BackoffConfig) -> Self {
        self.backoff = backoff;
        self
    }

    pub fn with_cancellation_token(mut self, cancellation_token: CancellationToken) -> Self {
        self.cancellation_token = cancellation_token;
        self
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    pub async fn run<R>(self, reconciler: R)
    where
        R: Reconciler<T>,
    {
        loop {
            let Some(stream) = self.watch_with_backoff().await else {
                return;
            };

            if !self.run_stream(stream, reconciler.clone()).await {
                return;
            }
        }
    }

    async fn watch_with_backoff(
        &self,
    ) -> Option<core::pin::Pin<Box<dyn Stream<Item = Result<WatchEvent<T>, Error>> + Send + '_>>>
    {
        let mut attempts = 0;
        loop {
            match self.api.watch(&self.watch_params).await {
                Ok(stream) => return Some(Box::pin(stream)),
                Err(err) => {
                    error!("failed to create {} watch stream: {err}", T::kind());
                    let delay = self.backoff.delay_for(attempts);
                    attempts = attempts.saturating_add(1);
                    tokio::select! {
                        _ = self.cancellation_token.cancelled() => return None,
                        _ = sleep(delay) => {}
                    }
                }
            }
        }
    }

    async fn run_stream<R, S>(&self, stream: S, reconciler: R) -> bool
    where
        R: Reconciler<T>,
        S: Stream<Item = Result<WatchEvent<T>, Error>> + Send,
    {
        futures::pin_mut!(stream);

        loop {
            let next = tokio::select! {
                _ = self.cancellation_token.cancelled() => return false,
                event = stream.next() => event,
            };

            match next {
                Some(Ok(event)) => {
                    self.spawn_reconcile(ReconcileEvent::from(event), reconciler.clone())
                }
                Some(Err(err)) => {
                    error!("{} watch stream failed: {err}", T::kind());
                    return true;
                }
                None => return !self.cancellation_token.is_cancelled(),
            }
        }
    }

    fn spawn_reconcile<R>(&self, event: ReconcileEvent<T>, reconciler: R)
    where
        R: Reconciler<T>,
    {
        let api = self.api.clone();
        let cancellation_token = self.cancellation_token.clone();
        tokio::spawn(async move {
            match reconciler.reconcile(event.clone()).await {
                Ok(action) => {
                    process_action(api, cancellation_token, reconciler, event, action).await;
                }
                Err(err) => {
                    error!("failed to reconcile {}: {err}", T::kind());
                }
            }
        });
    }
}

async fn process_action<T, R>(
    api: Api<T>,
    cancellation_token: CancellationToken,
    reconciler: R,
    event: ReconcileEvent<T>,
    action: Action,
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
    let Some(mut delay) = action.requeue_after() else {
        return;
    };
    let Some(name) = event.resource_name().map(ToOwned::to_owned) else {
        return;
    };

    loop {
        wait_or_cancel(&cancellation_token, delay).await;
        if cancellation_token.is_cancelled() {
            return;
        }

        let latest = match api.get(&name).await {
            Ok(Some(resource)) => resource,
            Ok(None) => return,
            Err(err) => {
                error!("failed to requeue {} {name}: {err}", T::kind());
                return;
            }
        };

        match reconciler.reconcile(ReconcileEvent::Applied(latest)).await {
            Ok(next_action) => {
                let Some(next_delay) = next_action.requeue_after() else {
                    return;
                };
                delay = next_delay;
            }
            Err(err) => {
                error!("failed to reconcile requeued {} {name}: {err}", T::kind());
                return;
            }
        }
    }
}

async fn wait_or_cancel(cancellation_token: &CancellationToken, delay: Duration) {
    tokio::select! {
        _ = cancellation_token.cancelled() => {}
        _ = sleep(delay) => {}
    }
}

trait ReconcileEventExt<T> {
    fn resource_name(&self) -> Option<&str>;
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
}

#[cfg(test)]
mod tests {
    use super::Controller;
    use crate::Api;
    use crate::runtime::{Action, ReconcileEvent};
    use crate::{TugboatClient, WatchEvent};
    use futures::stream;
    use std::convert::Infallible;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tugboat_resources::ObjectMetaResource;
    use tugboat_resources::manifests::core::v1::Ship;
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[tokio::test]
    async fn controller_processes_single_stream() {
        let api = Api::<Ship>::namespaced(TugboatClient::new("http://127.0.0.1:1"), "default");
        let controller = Controller::new(api);
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_clone = seen.clone();

        let stream = stream::iter([Ok(WatchEvent::Added(Ship {
            object_meta: Some(ObjectMeta {
                name: Some("demo".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }))]);

        controller
            .run_stream(stream, move |event: ReconcileEvent<Ship>| {
                let seen = seen_clone.clone();
                async move {
                    seen.lock().await.push(match event {
                        ReconcileEvent::Applied(resource) => resource.name().unwrap().to_string(),
                        ReconcileEvent::Deleted(resource) => resource.name().unwrap().to_string(),
                    });
                    Ok::<_, Infallible>(Action::await_change())
                }
            })
            .await;

        tokio::task::yield_now().await;

        assert_eq!(seen.lock().await.as_slice(), ["demo"]);
    }
}
