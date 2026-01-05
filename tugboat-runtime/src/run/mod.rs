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
