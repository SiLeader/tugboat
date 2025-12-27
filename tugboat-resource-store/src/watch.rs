use etcd_client::{Client, EventType};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::watch::{Receiver, Sender, channel};
use tokio::time::sleep;
use tracing::error;

pub struct KeyValue {
    pub key: String,
    pub value: Vec<u8>,
}

pub enum WatchEvent {
    Update(KeyValue),
    Delete(KeyValue),
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

    pub(crate) async fn get(&self, key: &str) -> Result<WatchReceiver, crate::Error> {
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
                let Ok((_watcher, mut stream)) = client.watch(key.as_str(), None).await else {
                    sleep(Duration::from_millis(500)).await;
                    continue;
                };
                while let Some(event) = stream.next().await {
                    match event {
                        Ok(event) => {
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
    let key = kv.key_str().ok()?.to_string();
    let kv = KeyValue {
        key,
        value: kv.value().to_vec(),
    };
    Some(match event.event_type() {
        EventType::Put => WatchEvent::Update(kv),
        EventType::Delete => WatchEvent::Delete(kv),
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
