use crate::operator::ApiOperator;
use tugboat_resource_store::ResourceStore;

#[derive(serde::Deserialize)]
pub struct ApiServerConfig {
    http: HttpConfig,
    etcd: EtcdConfig,
}

#[derive(serde::Deserialize)]
pub(crate) struct EtcdConfig {
    endpoints: Vec<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct HttpConfig {
    listen: String,
    mount: String,
    tls: Option<TlsConfig>,
}

#[derive(serde::Deserialize)]
pub struct TlsConfig {
    pub(crate) cert_file: String,
    pub(crate) key_file: String,
    pub(crate) client_cert_file: Option<String>,
}

impl crate::ApiServer {
    pub async fn from_config(value: ApiServerConfig) -> Self {
        let operator = ApiOperator::new(ResourceStore::new(value.etcd.endpoints.as_slice()).await);
        Self::new(
            value.http.listen,
            value.http.mount,
            operator,
            value.http.tls,
        )
    }
}

impl ApiServerConfig {
    pub fn load_from_file_or_panic(file: impl AsRef<std::path::Path>) -> Self {
        let file = std::fs::read_to_string(file).expect("Failed to read config file");
        toml::from_str(&file).expect("Failed to parse config file")
    }
}
