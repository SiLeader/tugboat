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

use crate::execute::vm::QemuVmConfig;

use crate::config::load_config;
use crate::pre::{create_and_enter_to_network_namespace, daemonize, enter_mount_namespace};
use clap::Parser;
use tugboat_vm_runtime_interface::run::VmRunRequest;

#[derive(Debug, Parser)]
pub(crate) struct StartArgs {
    #[arg(help = "Path to the VM config file")]
    config: String,
}

pub(crate) async fn run(vm: QemuVmConfig, args: StartArgs) -> Result<(), crate::Error> {
    let config = load_config::<VmRunRequest>(args.config)?;
    enter_mount_namespace(&config.id)?;
    create_and_enter_to_network_namespace(&config.id)?;
    daemonize();
    crate::execute::run(vm, config).await?;
    Ok(())
}
