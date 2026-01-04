mod config;
mod vm;

pub use vm::QemuVmConfig;

use crate::start::config::VmConfig;
use crate::start::vm::{QemuVmBuilder, Spawner};
use clap::Parser;

#[derive(Debug, Parser)]
pub(crate) struct StartArgs {
    #[arg(help = "Path to the VM config file")]
    config: String,
}

pub(crate) async fn start(vm: QemuVmConfig, args: StartArgs) {
    let config = VmConfig::load_or_panic(args.config);

    let spawner = QemuVmBuilder::new(vm);
    spawner.spawn(config).await.expect("Failed to spawn VM");
}
