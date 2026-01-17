use crate::execute::vm::{QemuVmBuilder, QemuVmConfig, Spawner};
use crate::pre::{change_running_user_and_group, daemonize};
use tugboat_vm_runtime_interface::run::VmRunRequest;

pub mod tap;
pub mod vm;

pub(crate) async fn run(vm: QemuVmConfig, config: VmRunRequest) -> Result<(), crate::Error> {
    // setup_tap_redirect(&vm, "").await?; // TODO bridge name
    daemonize();
    change_running_user_and_group(&config.user)?;

    let spawner = QemuVmBuilder::new(vm);
    spawner.spawn(config).await?;

    Ok(())
}
