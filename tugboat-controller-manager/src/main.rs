use crate::manager::TugboatControllerManager;
use tracing_subscriber::EnvFilter;

mod base;
mod manager;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let mut tcm = TugboatControllerManager::new();
    tcm.setup().await;
    tcm.run().await;
}
