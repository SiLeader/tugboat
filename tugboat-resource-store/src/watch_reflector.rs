use crate::watch::WatchEvent;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub(crate) struct WatchReflector {
    history: Arc<RwLock<BTreeMap<i64, WatchEvent>>>,
}
