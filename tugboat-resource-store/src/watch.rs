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
        _resource_version: Option<String>,
    ) -> Result<WatchReceiver, crate::Error> {
        debug!("Get watch receiver: key: {key}");
        let mut mux = self.mux.lock().await;
        if let Some(mux) = mux.get(key).cloned() {
            return Ok(mux.receiver());
        }
        let m = Arc::new(WatchMux::new());
        mux.insert(key.to_string(), m.clone());

        let client = self.client.watch_client();
        let key = key.to_string();
        let mux = m.clone();
        tokio::spawn(async move {
            let mut client = client;
            loop {
                let Ok((_watcher, mut stream)) = client
                    .watch(key.as_str(), Some(WatchOptions::default().with_prefix()))
                    .await
                else {
                    sleep(Duration::from_millis(500)).await;
                    continue;
                };
                while let Some(event) = stream.next().await {
                    match event {
                        Ok(event) => {
                            debug!("Watch event: key: {key}: {event:?}");
                            let events = event
                                .events()
                                .iter()
                                .filter_map(transform_event)
                                .collect::<Vec<_>>();
                            if let Err(e) = mux.emit(events) {
                                error!("Watch event emit error: {e}");
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
        Ok(m.receiver())
    }
}

fn transform_event(event: &etcd_client::Event) -> Option<WatchEvent> {
    let kv = event.kv()?;
    let is_added = kv.version() == 1;
    let key = kv.key_str().ok()?.to_string();
    let kv = KeyValue {
        key,
        value: kv.value().to_vec(),
    };
    Some(match event.event_type() {
        EventType::Put => {
            if is_added {
                WatchEvent::Added(kv)
            } else {
                WatchEvent::Modified(kv)
            }
        }
        EventType::Delete => WatchEvent::Deleted(kv),
    })
}

pub(crate) struct WatchMux {
    tx: Sender<Vec<WatchEvent>>,
    rx: Receiver<Vec<WatchEvent>>,
}

pub type WatchReceiver = Receiver<Vec<WatchEvent>>;

impl WatchMux {
    pub(crate) fn new() -> Self {
        let (tx, rx) = channel(vec![]);
        Self { tx, rx }
    }

    pub(crate) fn emit(&self, value: Vec<WatchEvent>) -> Result<(), crate::Error> {
        self.tx.send(value)?;
        Ok(())
    }

    pub(crate) fn receiver(&self) -> WatchReceiver {
        self.rx.clone()
    }
}

impl Default for WatchMux {
    fn default() -> Self {
        Self::new()
    }
}
