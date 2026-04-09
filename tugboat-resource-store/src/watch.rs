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

    // resource_version is not used yet.
    pub(crate) async fn get(
        &self,
        key: &str,
        resource_version: Option<String>,
    ) -> Result<WatchReceiver, crate::Error> {
        debug!("Get watch receiver: key: {key}");
        let mut mux_map = self.mux.lock().await;
        if let Some(mux) = mux_map.get(key).cloned()
            && resource_version.is_none()
        {
            return Ok(mux.receiver());
        }
        let m = Arc::new(WatchMux::new());
        if resource_version.is_none() {
            mux_map.insert(key.to_string(), m.clone());
        }
        drop(mux_map);

        let client = self.client.watch_client();
        let key = key.to_string();
        let watch_mux = m.clone();
        let aggregator_mux = Arc::clone(&self.mux);
        let start_revision = resource_version
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .map(|revision| revision.saturating_add(1));
        // Create the receiver before spawning to avoid a race where the task
        // sees zero receivers and exits before the caller can subscribe.
        let receiver = m.receiver();
        tokio::spawn(async move {
            let mut client = client;
            let mut current_revision = start_revision;
            loop {
                if !watch_mux.has_receivers() {
                    debug!("No active receivers for watch key: {key}, stopping watch task");
                    aggregator_mux.lock().await.remove(&key);
                    break;
                }
                let mut options = WatchOptions::default().with_prefix().with_prev_key();
                if let Some(rev) = current_revision {
                    options = options.with_start_revision(rev);
                }
                let Ok(mut stream) = client.watch(key.as_str(), Some(options)).await else {
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
                                debug!(
                                    "All receivers dropped for watch key: {key}, \
                                     stopping watch task"
                                );
                                aggregator_mux.lock().await.remove(&key);
                                return;
                            }
                        }
                        Err(e) => {
                            error!("Watch error: key: {key}: {e}");
                            break;
                        }
                    }
                }
            }
        });
        Ok(receiver)
    }
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
