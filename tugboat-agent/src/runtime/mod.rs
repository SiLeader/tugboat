pub(crate) mod error;
mod run;
mod runtime;

use crate::runtime::runtime::Runtime;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tugboat_vm_image::VmImageRegistry;

#[derive(Clone)]
pub(crate) struct RuntimeOperator {
    config: RuntimeConfig,
    registry: VmImageRegistry,
    children: Arc<RwLock<HashMap<String, Runtime>>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RuntimeConfig {
    pub executable: String,
    pub args: Vec<String>,
    pub config_file: String,
}

impl RuntimeOperator {}
