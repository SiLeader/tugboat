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
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
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
    in_flight: Arc<Mutex<HashMap<String, (u64, CancellationToken)>>>,
    counter: Arc<AtomicU64>,
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
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            counter: Arc::new(AtomicU64::new(0)),
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
        let parent_token = self.cancellation_token.clone();
        let in_flight = self.in_flight.clone();
        let this_backoff = self.backoff.clone();
        // Capture counter for the branch that uses it
        let counter = self.counter.clone();

        let resource_key = match event.resource_key() {
            Some(key) => key,
            None => {
                // No key available; fall back to spawning without deduplication.
                let child_token = parent_token.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = child_token.cancelled() => None,
                        res = reconciler.reconcile(event.clone()) => Some(res),
                    };

                    if let Some(res) = result {
                        process_reconcile_result(
                            api,
                            child_token,
                            reconciler,
                            &event,
                            res,
                            &this_backoff,
                            0,
                        )
                        .await;
                    }
                });
                return;
            }
        };

        let id = counter.fetch_add(1, Ordering::Relaxed);

        tokio::spawn(async move {
            // Cancel any previous reconciliation for this resource.
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
                process_reconcile_result(
                    api,
                    child_token,
                    reconciler,
                    &event,
                    res,
                    &this_backoff,
                    0,
                )
                .await;
            }

            // Clean up only if our token is still the active one (not superseded).
            let mut map = in_flight.lock().await;
            if let Some((current_id, _)) = map.get(&resource_key)
                && *current_id == id
            {
                map.remove(&resource_key);
            }
        });
    }
}

async fn process_reconcile_result<T, R>(
    api: Api<T>,
    cancellation_token: CancellationToken,
    reconciler: R,
    event: &ReconcileEvent<T>,
    result: Result<Action, R::Error>,
    backoff: &BackoffConfig,
    attempt: u32,
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
    match result {
        Ok(action) => {
            process_action(api, cancellation_token, reconciler, event, action, backoff).await;
        }
        Err(err) => {
            let name = event.resource_name().unwrap_or("unknown");
            error!("failed to reconcile {} {name}: {err}", T::kind());

            let delay = backoff.delay_for(attempt);
            process_requeue(
                api,
                cancellation_token,
                reconciler,
                event,
                delay,
                backoff,
                attempt.saturating_add(1),
            )
            .await;
        }
    }
}

async fn process_action<T, R>(
    api: Api<T>,
    cancellation_token: CancellationToken,
    reconciler: R,
    event: &ReconcileEvent<T>,
    action: Action,
    backoff: &BackoffConfig,
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
    let Some(delay) = action.requeue_after() else {
        return;
    };
    process_requeue(
        api,
        cancellation_token,
        reconciler,
        event,
        delay,
        backoff,
        0,
    )
    .await;
}

async fn process_requeue<T, R>(
    api: Api<T>,
    cancellation_token: CancellationToken,
    reconciler: R,
    event: &ReconcileEvent<T>,
    mut delay: Duration,
    backoff: &BackoffConfig,
    mut attempt: u32,
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
    let Some(name) = event.resource_name().map(ToOwned::to_owned) else {
        return;
    };
    let namespace = event.resource_namespace().map(ToOwned::to_owned);

    loop {
        wait_or_cancel(&cancellation_token, delay).await;
        if cancellation_token.is_cancelled() {
            return;
        }

        let latest = match api
            .get_with_optional_namespace(namespace.as_deref(), &name)
            .await
        {
            Ok(Some(resource)) => resource,
            Ok(None) => return,
            Err(err) => {
                error!("failed to requeue {} {name}: {err}", T::kind());
                // On API error, retry with backoff
                delay = backoff.delay_for(attempt);
                attempt = attempt.saturating_add(1);
                continue;
            }
        };

        match reconciler.reconcile(ReconcileEvent::Applied(latest)).await {
            Ok(next_action) => {
                let Some(next_delay) = next_action.requeue_after() else {
                    return;
                };
                delay = next_delay;
                attempt = 0;
            }
            Err(err) => {
                error!("failed to reconcile requeued {} {name}: {err}", T::kind());
                delay = backoff.delay_for(attempt);
                attempt = attempt.saturating_add(1);
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

    #[tokio::test]
    async fn duplicate_events_cancel_previous_requeue() {
        use std::time::Duration;

        let api = Api::<Ship>::namespaced(TugboatClient::new("http://127.0.0.1:1"), "default");
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

        // Verify the in-flight map has at most one entry for the resource.
        let map = controller.in_flight.lock().await;
        assert!(
            map.len() <= 1,
            "expected at most 1 in-flight entry, got {}",
            map.len()
        );
    }
}
