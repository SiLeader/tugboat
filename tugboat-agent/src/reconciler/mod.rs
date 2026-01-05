mod reconcile;

use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::core::v1::Ship;

#[derive(Clone)]
pub(crate) struct ShipReconciler {
    api: Api<Ship>,
}

impl ShipReconciler {
    pub(crate) fn new(client: TugboatClient) -> Self {
        Self {
            api: Api::all(client),
        }
    }

    pub(crate) async fn run(self) {
        loop {
            todo!()
        }
    }
}
