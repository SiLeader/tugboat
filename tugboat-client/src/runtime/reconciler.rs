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

use crate::runtime::Action;
use async_trait::async_trait;
use std::future::Future;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconcileEvent<T> {
    Applied(T),
    Deleted(T),
}

impl<T> From<crate::WatchEvent<T>> for ReconcileEvent<T> {
    fn from(value: crate::WatchEvent<T>) -> Self {
        match value {
            crate::WatchEvent::Added(resource) | crate::WatchEvent::Modified(resource) => {
                Self::Applied(resource)
            }
            crate::WatchEvent::Deleted(resource) => Self::Deleted(resource),
        }
    }
}

#[async_trait]
pub trait Reconciler<T>: Clone + Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn reconcile(&self, event: ReconcileEvent<T>) -> Result<Action, Self::Error>;
}

#[async_trait]
impl<T, E, F, Fut> Reconciler<T> for F
where
    T: Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
    F: Fn(ReconcileEvent<T>) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Action, E>> + Send + 'static,
{
    type Error = E;

    async fn reconcile(&self, event: ReconcileEvent<T>) -> Result<Action, Self::Error> {
        (self)(event).await
    }
}
