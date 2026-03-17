#[async_trait::async_trait]
pub(crate) trait TugboatController: Send + Sync {
    fn name(&self) -> &str;
    async fn setup(&mut self);
    async fn run(&self);
}
