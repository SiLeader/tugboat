use rtnetlink::NetworkNamespace;

pub(crate) struct TapOperator {}

impl TapOperator {
    pub async fn create_namespace(&self, ns: impl ToString) -> Result<(), crate::Error> {
        NetworkNamespace::add(ns.to_string()).await?;
        Ok(())
    }

    pub async fn create_tap(&self, tap_name: &str) -> Result<(), crate::Error> {
        tokio_tun::Tun::builder()
            .name(tap_name)
            .tap()
            .persist()
            .up()
            .build()?;
    }

    pub async fn setup_ingress(&self, tap_name: &str, cni_bridge_name: &str) {}
}
