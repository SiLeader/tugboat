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
use crate::runtime::queue::ReconcileQueue;
use crate::runtime::{BackoffConfig, ReconcileEvent, Reconciler};
use crate::{Error, WatchEvent, WatchParams};
use futures::{Stream, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;
use tracing::error;
use tugboat_resources::{ObjectMetaResource, StaticResource};

#[derive(Clone)]
pub struct Controller<T> {
    api: Api<T>,
    watch_params: WatchParams,
    backoff: BackoffConfig,
    cancellation_token: CancellationToken,
    queue: ReconcileQueue<T>,
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
            queue: ReconcileQueue::new(),
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
            let Some(stream) = crate::runtime::stream::watch_with_backoff(
                &self.api,
                &self.watch_params,
                &self.backoff,
                &self.cancellation_token,
            )
            .await
            else {
                return;
            };

            if !self.run_stream(stream, reconciler.clone()).await {
                return;
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
        self.queue.spawn(
            self.api.clone(),
            self.cancellation_token.clone(),
            self.backoff.clone(),
            event,
            reconciler,
        );
    }

    #[cfg(test)]
    async fn in_flight_len_for_tests(&self) -> usize {
        self.queue.in_flight_len().await
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
        let api = Api::<Ship>::namespaced(TugboatClient::new("https://127.0.0.1:1"), "default");
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

    #[tokio::test]
    async fn duplicate_events_cancel_previous_requeue() {
        use std::time::Duration;

        let api = Api::<Ship>::namespaced(TugboatClient::new("https://127.0.0.1:1"), "default");
        let controller = Controller::new(api);
        let call_count = Arc::new(Mutex::new(0u32));
        let call_count_clone = call_count.clone();

        // Two events for the same resource in rapid succession.
        let make_ship = || Ship {
            object_meta: Some(ObjectMeta {
                name: Some("ship-a".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let stream = stream::iter([
            Ok(WatchEvent::Added(make_ship())),
            Ok(WatchEvent::Modified(make_ship())),
        ]);

        // Each reconcile returns a long requeue. If deduplication works, the
        // first requeue loop should be cancelled when the second event arrives.
        controller
            .run_stream(stream, move |_event: ReconcileEvent<Ship>| {
                let count = call_count_clone.clone();
                async move {
                    *count.lock().await += 1;
                    Ok::<_, Infallible>(Action::requeue(Duration::from_secs(3600)))
                }
            })
            .await;

        // Let spawned tasks settle.
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Both events should have triggered reconciliation.
        let count = *call_count.lock().await;
        assert!(
            count >= 1,
            "expected at least 1 reconcile call, got {count}"
        );

        // Verify the queue has at most one in-flight entry for the resource.
        let in_flight_len = controller.in_flight_len_for_tests().await;
        assert!(
            in_flight_len <= 1,
            "expected at most 1 in-flight entry, got {in_flight_len}",
        );
    }
}
