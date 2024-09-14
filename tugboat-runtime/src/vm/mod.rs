use async_trait::async_trait;
use tokio::process::Child;

mod qemu;

#[async_trait]
pub(crate) trait RunVm {
    async fn run_vm(&self) -> crate::Result<Child>;
}
