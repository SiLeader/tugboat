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
use crate::runtime::queue::ReconcileEventExt;
use crate::runtime::{Action, BackoffConfig, ReconcileEvent, Reconciler};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::error;
use tugboat_resources::{ObjectMetaResource, StaticResource};

pub(super) async fn process_reconcile_result<T, R>(
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
