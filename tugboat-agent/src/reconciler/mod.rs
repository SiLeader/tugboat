mod error;
mod reconcile;

use crate::runtime::RuntimeOperator;
use futures::{Stream, StreamExt};
use std::cmp::min;
use std::pin::Pin;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info};
use tugboat_client::{Api, TugboatClient, WatchEvent, WatchParams};
use tugboat_resources::manifests::core::v1::{Ship, ShipClass};

#[derive(Clone)]
pub(crate) struct ShipReconciler {
    node_name: String,
    api: Api<Ship>,
    ship_class_api: Api<ShipClass>,
    runtime_operator: RuntimeOperator,
}

impl ShipReconciler {
    pub(crate) fn new(
        node_name: String,
        client: TugboatClient,
        runtime_operator: RuntimeOperator,
    ) -> Self {
        Self {
            node_name,
            api: Api::all(client.clone()),
            ship_class_api: Api::all(client),
            runtime_operator,
        }
    }

    pub(crate) async fn run(self) {
        info!(
            "Starting ship reconciliation loop on node '{}'",
            self.node_name
        );
        let watch_params =
            WatchParams::default().fields(format!("spec.nodeName={}", self.node_name));

        loop {
            let mut stream = self.get_watch_stream(&watch_params).await;
            while let Some(event) = stream.next().await {
                match event {
                    Ok(event) => {
                        if let Err(e) = self.reconcile(event).await {
                            error!("Failed to reconcile ship: {e}");
                        }
                    }
                    Err(err) => {
                        error!("Failed to watch ship: {err}");
                    }
                }
            }
        }
    }

    async fn get_watch_stream(
        &self,
        params: &WatchParams,
    ) -> Pin<Box<impl Stream<Item = Result<WatchEvent<Ship>, tugboat_client::Error>>>> {
        let mut count = 0;
        loop {
            match self.api.watch(params).await {
                Ok(stream) => return Box::pin(stream),
                Err(e) => {
                    error!("Failed to create watch stream: {e}");
                    sleep(Duration::from_secs(min(128, 2u64.pow(count)))).await;
                    count += 1;
                }
            }
        }
    }
}
