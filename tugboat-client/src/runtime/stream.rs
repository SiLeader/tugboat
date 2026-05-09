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
use crate::runtime::BackoffConfig;
use crate::{Error, WatchEvent, WatchParams};
use futures::Stream;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::error;
use tugboat_resources::{ObjectMetaResource, StaticResource};

pub(super) async fn watch_with_backoff<'a, T>(
    api: &'a Api<T>,
    watch_params: &'a WatchParams,
    backoff: &'a BackoffConfig,
    cancellation_token: &'a CancellationToken,
) -> Option<core::pin::Pin<Box<dyn Stream<Item = Result<WatchEvent<T>, Error>> + Send + 'a>>>
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
    let mut attempts = 0;
    loop {
        match api.watch(watch_params).await {
            Ok(stream) => return Some(Box::pin(stream)),
            Err(err) => {
                error!("failed to create {} watch stream: {err}", T::kind());
                let delay = backoff.delay_for(attempts);
                attempts = attempts.saturating_add(1);
                tokio::select! {
                    _ = cancellation_token.cancelled() => return None,
                    _ = sleep(delay) => {}
                }
            }
        }
    }
}
