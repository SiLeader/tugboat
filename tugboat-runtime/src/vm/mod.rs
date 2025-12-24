use crate::RuntimeArgs;
use async_trait::async_trait;
pub use qemu::*;

mod qemu;

#[async_trait]
pub(crate) trait RunVm {
    async fn run_vm(&self) -> crate::Result<()>;
}

#[async_trait]
pub trait Spawner {
    async fn spawn(&self, args: RuntimeArgs) -> crate::Result<()>;
}
