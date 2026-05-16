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

use etcd_client::{Client, EventType, WatchOptions};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::watch::{Receiver, Sender, channel};
use tokio::time::sleep;
use tracing::{debug, error};

#[derive(Clone)]
pub struct KeyValue {
    pub key: String,
    pub value: Vec<u8>,
    pub revision: i64,
}

#[derive(Clone)]
pub enum WatchEvent {
    Added(KeyValue),
    Modified(KeyValue),
    Deleted(KeyValue),
}

#[derive(Clone)]
pub(crate) struct WatchMuxAggregator {
    client: Client,
    mux: Arc<Mutex<HashMap<String, Arc<WatchMux>>>>,
}

impl WatchMuxAggregator {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            mux: Default::default(),
        }
    }

    pub(crate) async fn get(
        &self,
        key: &str,
        resource_version: Option<String>,
    ) -> Result<WatchReceiver, crate::Error> {
        let start_revision = parse_resource_version(resource_version.as_deref())?
            .map(|revision| revision.saturating_add(1));
        self.get_from_start_revision(key, start_revision).await
    }

    pub(crate) async fn get_from_start_revision(
        &self,
        key: &str,
        start_revision: Option<i64>,
    ) -> Result<WatchReceiver, crate::Error> {
        debug!("Get watch receiver: key: {key}");
        if let Some(revision) = start_revision
            && revision < 0
        {
            return Err(crate::Error::InvalidField(
                "startRevision".to_string(),
                "must not be negative".to_string(),
            ));
        }
        let mut mux_map = self.mux.lock().await;
        if let Some(mux) = mux_map.get(key).cloned()
            && start_revision.is_none()
        {
            return Ok(mux.receiver());
        }
        let m = Arc::new(WatchMux::new());
        if start_revision.is_none() {
            mux_map.insert(key.to_string(), m.clone());
        }
        drop(mux_map);

        let client = self.client.watch_client();
        let key = key.to_string();
        let watch_mux = m.clone();
        let aggregator_mux = Arc::clone(&self.mux);
        // Create the receiver before spawning to avoid a race where the task
        // sees zero receivers and exits before the caller can subscribe.
        let receiver = m.receiver();
        tokio::spawn(async move {
            let mut client = client;
            let mut current_revision = start_revision;
            loop {
                if stop_watch_if_unused(&watch_mux, &aggregator_mux, &key).await {
                    break;
                }
                let mut options = WatchOptions::default().with_prefix().with_prev_key();
                if let Some(rev) = current_revision {
                    options = options.with_start_revision(rev);
                }
                let Ok(mut stream) = client.watch(key.as_str(), Some(options)).await else {
                    if stop_watch_if_unused(&watch_mux, &aggregator_mux, &key).await {
                        break;
                    }
                    sleep(Duration::from_millis(500)).await;
                    continue;
                };
                while let Some(event) = stream.next().await {
                    match event {
                        Ok(event) => {
                            debug!("Watch event: key: {key}: {event:?}");
                            if let Some(header) = event.header() {
                                current_revision = Some(header.revision() + 1);
                            }
                            let events = event
                                .events()
                                .iter()
                                .filter_map(transform_event)
                                .collect::<Vec<_>>();
                            if watch_mux.emit(events).is_err() {
                                cleanup_watch(&aggregator_mux, &key).await;
                                return;
                            }
                        }
                        Err(e) => {
                            error!("Watch error: key: {key}: {e}");
                            break;
                        }
                    }
                }

                if stop_watch_if_unused(&watch_mux, &aggregator_mux, &key).await {
                    break;
                }
            }
        });
        Ok(receiver)
    }
}

fn parse_resource_version(resource_version: Option<&str>) -> Result<Option<i64>, crate::Error> {
    let Some(value) = resource_version else {
        return Ok(None);
    };
    let revision = value.parse::<i64>().map_err(|_| {
        crate::Error::InvalidField(
            "resourceVersion".to_string(),
            "must be a valid integer".to_string(),
        )
    })?;
    if revision < 0 {
        return Err(crate::Error::InvalidField(
            "resourceVersion".to_string(),
            "must not be negative".to_string(),
        ));
    }
    Ok(Some(revision))
}

async fn stop_watch_if_unused(
    watch_mux: &WatchMux,
    aggregator_mux: &Arc<Mutex<HashMap<String, Arc<WatchMux>>>>,
    key: &str,
) -> bool {
    if watch_mux.has_receivers() {
        return false;
    }

    debug!("No active receivers for watch key: {key}, stopping watch task");
    cleanup_watch(aggregator_mux, key).await;
    true
}

async fn cleanup_watch(aggregator_mux: &Arc<Mutex<HashMap<String, Arc<WatchMux>>>>, key: &str) {
    aggregator_mux.lock().await.remove(key);
}

fn transform_event(event: &etcd_client::Event) -> Option<WatchEvent> {
    match event.event_type() {
        EventType::Put => {
            let kv = event.kv()?;
            let is_added = kv.version() == 1;
            let key = kv.key_str().ok()?.to_string();
            let kv = KeyValue {
                key,
                value: kv.value().to_vec(),
                revision: kv.mod_revision(),
            };
            Some(if is_added {
                WatchEvent::Added(kv)
            } else {
                WatchEvent::Modified(kv)
            })
        }
        EventType::Delete => {
            let kv = event.prev_kv().or_else(|| event.kv())?;
            let key = kv.key_str().ok()?.to_string();
            Some(WatchEvent::Deleted(KeyValue {
                key,
                value: kv.value().to_vec(),
                revision: kv.mod_revision(),
            }))
        }
    }
}

pub(crate) struct WatchMux {
    tx: Sender<Vec<WatchEvent>>,
}

pub type WatchReceiver = Receiver<Vec<WatchEvent>>;

impl WatchMux {
    pub(crate) fn new() -> Self {
        let (tx, _) = channel(vec![]);
        Self { tx }
    }

    pub(crate) fn emit(&self, value: Vec<WatchEvent>) -> Result<(), crate::Error> {
        self.tx.send(value)?;
        Ok(())
    }

    pub(crate) fn receiver(&self) -> WatchReceiver {
        self.tx.subscribe()
    }

    pub(crate) fn has_receivers(&self) -> bool {
        self.tx.receiver_count() > 0
    }
}

impl Default for WatchMux {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{WatchMux, cleanup_watch, parse_resource_version};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[test]
    fn watch_mux_tracks_receivers() {
        let mux = WatchMux::new();
        assert!(!mux.has_receivers());

        let receiver = mux.receiver();
        assert!(mux.has_receivers());

        drop(receiver);
        assert!(!mux.has_receivers());
    }

    #[test]
    fn emit_fails_when_no_receivers_exist() {
        let mux = WatchMux::new();

        let result = mux.emit(vec![]);

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cleanup_watch_removes_mux_entry() {
        let mux_map = Arc::new(Mutex::new(HashMap::new()));
        mux_map
            .lock()
            .await
            .insert("/demo".to_string(), Arc::new(WatchMux::new()));

        cleanup_watch(&mux_map, "/demo").await;

        assert!(!mux_map.lock().await.contains_key("/demo"));
    }

    #[test]
    fn resource_version_none_starts_from_current_revision() {
        assert_eq!(parse_resource_version(None).unwrap(), None);
    }

    #[test]
    fn resource_version_parses_as_etcd_revision() {
        assert_eq!(parse_resource_version(Some("42")).unwrap(), Some(42));
    }

    #[test]
    fn resource_version_rejects_non_numeric_values() {
        let err = parse_resource_version(Some("latest")).unwrap_err();

        assert!(matches!(
            err,
            crate::Error::InvalidField(ref field, _) if field == "resourceVersion"
        ));
    }

    #[test]
    fn resource_version_rejects_negative_values() {
        let err = parse_resource_version(Some("-1")).unwrap_err();

        assert!(matches!(
            err,
            crate::Error::InvalidField(ref field, _) if field == "resourceVersion"
        ));
    }
}
