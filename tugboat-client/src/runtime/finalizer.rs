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
use crate::runtime::Action;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::future::Future;
use thiserror::Error;
use tugboat_resources::{ObjectMetaResource, StaticResource};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalizerEvent<T> {
    Apply(T),
    Cleanup(T),
}

#[derive(Debug, Error)]
pub enum FinalizerError<E>
where
    E: std::error::Error + 'static,
{
    #[error("client error: {0}")]
    Client(#[from] Error),
    #[error("finalizer handler error: {0}")]
    Handler(E),
    #[error("resource metadata.name is missing")]
    MissingName,
}

enum FinalizerTransition {
    AddFinalizer,
    Apply,
    Cleanup,
    Noop,
}

pub async fn finalizer<T, E, F, Fut>(
    api: &Api<T>,
    finalizer_name: &str,
    resource: T,
    reconcile: F,
) -> Result<Action, FinalizerError<E>>
where
    T: StaticResource + ObjectMetaResource + Serialize + DeserializeOwned + Clone,
    E: std::error::Error + 'static,
    F: FnOnce(FinalizerEvent<T>) -> Fut,
    Fut: Future<Output = Result<Action, E>>,
{
    let name = resource
        .name()
        .map(ToOwned::to_owned)
        .ok_or(FinalizerError::MissingName)?;

    match transition_for(&resource, finalizer_name) {
        FinalizerTransition::AddFinalizer => {
            let mut updated = resource;
            updated.add_finalizer(finalizer_name);
            api.replace(&name, updated).await?;
            Ok(Action::requeue_immediately())
        }
        FinalizerTransition::Apply => reconcile(FinalizerEvent::Apply(resource))
            .await
            .map_err(FinalizerError::Handler),
        FinalizerTransition::Cleanup => {
            let action = reconcile(FinalizerEvent::Cleanup(resource.clone()))
                .await
                .map_err(FinalizerError::Handler)?;

            let mut updated = resource;
            updated.remove_finalizer(finalizer_name);
            let updated = api.replace(&name, updated).await?;
            if updated.deletion_timestamp().is_some() && !updated.has_finalizers() {
                match api.delete(&name).await {
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(
                            "failed to delete {} {name} after finalizer removal: {err}",
                            T::kind()
                        );
                    }
                }
            }
            Ok(action)
        }
        FinalizerTransition::Noop => Ok(Action::await_change()),
    }
}

fn transition_for<T: ObjectMetaResource>(
    resource: &T,
    finalizer_name: &str,
) -> FinalizerTransition {
    if resource.deletion_timestamp().is_some() {
        if resource.has_finalizer(finalizer_name) {
            FinalizerTransition::Cleanup
        } else {
            FinalizerTransition::Noop
        }
    } else if resource.has_finalizer(finalizer_name) {
        FinalizerTransition::Apply
    } else {
        FinalizerTransition::AddFinalizer
    }
}

#[cfg(test)]
mod tests {
    use super::{FinalizerTransition, transition_for};
    use tugboat_resources::ObjectMetaResource;
    use tugboat_resources::manifests::core::v1::Ship;
    use tugboat_resources::manifests::meta::v1::Time;

    #[test]
    fn transitions_to_add_before_reconcile() {
        let resource = Ship::default();
        assert!(matches!(
            transition_for(&resource, "example.com/finalizer"),
            FinalizerTransition::AddFinalizer
        ));
    }

    #[test]
    fn transitions_to_apply_when_finalizer_is_present() {
        let mut resource = Ship::default();
        resource.add_finalizer("example.com/finalizer");

        assert!(matches!(
            transition_for(&resource, "example.com/finalizer"),
            FinalizerTransition::Apply
        ));
    }

    #[test]
    fn transitions_to_cleanup_when_marked_for_deletion() {
        let mut resource = Ship::default();
        resource.add_finalizer("example.com/finalizer");
        resource.mark_for_deletion(Time::now());

        assert!(matches!(
            transition_for(&resource, "example.com/finalizer"),
            FinalizerTransition::Cleanup
        ));
    }
}
