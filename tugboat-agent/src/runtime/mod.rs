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
