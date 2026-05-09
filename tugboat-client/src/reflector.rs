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
use crate::error::Error;
use crate::watch::{WatchEvent, WatchParams};
use async_stream::stream;
use futures::{Stream, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tugboat_resources::StaticResource;

impl<T> Api<T>
where
    T: StaticResource + Serialize + DeserializeOwned,
{
    /// First lists existing resources (emitted as `WatchEvent::Added`),
    /// then streams subsequent watch events.
    pub async fn reflector(
        &self,
        params: &WatchParams,
    ) -> Result<impl Stream<Item = Result<WatchEvent<T>, Error>>, Error> {
        let initial_list = self.list_with_params_full(params).await?;
        let mut watch_params = params.clone();
        apply_list_resource_version(
            &mut watch_params,
            initial_list.metadata.resource_version.as_deref(),
        );

        let watch_stream = self.watch_raw(watch_params).await?;

        Ok(stream! {
            for item in initial_list.items {
                yield Ok(WatchEvent::Added(item));
            }
            futures::pin_mut!(watch_stream);
            while let Some(event) = watch_stream.next().await {
                yield event;
            }
        })
    }

    pub async fn watch(
        &self,
        params: &WatchParams,
    ) -> Result<impl Stream<Item = Result<WatchEvent<T>, Error>>, Error> {
        self.reflector(params).await
    }
}

fn apply_list_resource_version(
    watch_params: &mut WatchParams,
    list_resource_version: Option<&str>,
) {
    if let Some(resource_version) = list_resource_version {
        watch_params.resource_version = Some(resource_version.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_explicit_resource_version_when_list_revision_is_absent() {
        let mut params = WatchParams::default().resource_version("42");

        apply_list_resource_version(&mut params, None);

        assert_eq!(params.resource_version.as_deref(), Some("42"));
    }

    #[test]
    fn uses_list_resource_version_when_available() {
        let mut params = WatchParams::default().resource_version("42");

        apply_list_resource_version(&mut params, Some("84"));

        assert_eq!(params.resource_version.as_deref(), Some("84"));
    }

    #[test]
    fn leaves_resource_version_absent_without_list_revision() {
        let mut params = WatchParams::default();

        apply_list_resource_version(&mut params, None);

        assert_eq!(params.resource_version, None);
    }
}
