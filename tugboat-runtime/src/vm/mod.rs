use async_trait::async_trait;

mod qemu;

pub(crate) enum Error {}

#[async_trait]
pub(crate) trait RunVm {
    async fn run_vm(&self);
}
