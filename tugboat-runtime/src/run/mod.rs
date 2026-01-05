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

mod config;
mod vm;

pub use vm::QemuVmConfig;

use crate::run::config::VmConfig;
use crate::run::vm::{QemuVmBuilder, Spawner};
use clap::Parser;

#[derive(Debug, Parser)]
pub(crate) struct RunArgs {
    #[arg(help = "Path to the VM config file")]
    config: String,
}

pub(crate) async fn run(vm: QemuVmConfig, args: RunArgs) {
    let config = VmConfig::load_or_panic(args.config);

    let spawner = QemuVmBuilder::new(vm);
    spawner.spawn(config).await.expect("Failed to spawn VM");
}
