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
    mux: Arc<Mutex<HashMap<String, Sender<Vec<WatchEvent>>>>>,
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
        debug!("Get watch receiver: key: {key}");
        let start_revision = parse_start_revision(resource_version.as_deref())?;
        if let Some(start_revision) = start_revision {
            let (tx, rx) = channel(vec![]);
            Self::spawn_watch_task(
                self.client.watch_client(),
                key.to_string(),
                tx,
                Some(start_revision),
                false,
            );
            return Ok(rx);
        }

        let mut mux = self.mux.lock().await;
        if let Some(tx) = mux.get(key) {
            return Ok(tx.subscribe());
        }

        let (tx, rx) = channel(vec![]);
        mux.insert(key.to_string(), tx.clone());
        Self::spawn_watch_task(
            self.client.watch_client(),
            key.to_string(),
            tx,
            None,
            true,
        );
        Ok(rx)
    }

    fn spawn_watch_task(
        client: etcd_client::WatchClient,
        key: String,
        tx: Sender<Vec<WatchEvent>>,
        start_revision: Option<i64>,
        keep_without_receivers: bool,
    ) {
        tokio::spawn(async move {
            let mut client = client;
            loop {
                let watch_options = build_watch_options(start_revision);
                let Ok((_watcher, mut stream)) =
                    client.watch(key.as_str(), Some(watch_options)).await
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
                            if tx.send(events).is_err() {
                                if keep_without_receivers {
                                    continue;
                                }
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
    }
}

fn parse_start_revision(resource_version: Option<&str>) -> Result<Option<i64>, crate::Error> {
    let Some(resource_version) = resource_version else {
        return Ok(None);
    };
    let revision = resource_version
        .parse::<i64>()
        .map_err(|_| crate::Error::InvalidResourceVersion(resource_version.to_string()))?;
    if revision < 0 {
        return Err(crate::Error::InvalidResourceVersion(
            resource_version.to_string(),
        ));
    }
    let start_revision = revision
        .checked_add(1)
        .ok_or_else(|| crate::Error::InvalidResourceVersion(resource_version.to_string()))?;
    Ok(Some(start_revision))
}

fn build_watch_options(start_revision: Option<i64>) -> WatchOptions {
    let options = WatchOptions::default().with_prefix().with_prev_key();
    if let Some(start_revision) = start_revision {
        options.with_start_revision(start_revision)
    } else {
        options
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
            }))
        }
    }
}

pub type WatchReceiver = Receiver<Vec<WatchEvent>>;

#[cfg(test)]
mod tests {
    use super::parse_start_revision;

    #[test]
    fn parse_start_revision_none() {
        let revision = parse_start_revision(None).unwrap();
        assert_eq!(revision, None);
    }

    #[test]
    fn parse_start_revision_valid() {
        let revision = parse_start_revision(Some("123")).unwrap();
        assert_eq!(revision, Some(124));
    }

    #[test]
    fn parse_start_revision_invalid_text() {
        assert!(parse_start_revision(Some("invalid")).is_err());
    }

    #[test]
    fn parse_start_revision_invalid_negative() {
        assert!(parse_start_revision(Some("-1")).is_err());
    }

    #[test]
    fn parse_start_revision_invalid_overflow() {
        let max = i64::MAX.to_string();
        assert!(parse_start_revision(Some(max.as_str())).is_err());
    }
}
