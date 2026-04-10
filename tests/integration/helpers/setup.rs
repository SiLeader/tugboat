#![allow(dead_code)]

use std::collections::HashMap;
use std::error::Error;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::x509::extension::{
    BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectAlternativeName,
};
use openssl::x509::{X509, X509NameBuilder};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing_subscriber::EnvFilter;
use tugboat_apiserver::ApiServer;
use tugboat_apiserver::config::ApiServerConfig;
use tugboat_client::TugboatClient;
use tugboat_resource_store::ResourceStore;
use tugboat_resources::Resource;
use tugboat_resources::manifests::authorization::v1::{ClusterRoleBinding, RoleRef, Subject};
use tugboat_resources::manifests::core::v1::{Namespace, Secret, ServiceAccount};
use tugboat_resources::manifests::meta::v1::ObjectMeta;

type DynError = Box<dyn Error + Send + Sync>;

const APISERVER_URL_ENV: &str = "TUGBOAT_TEST_APISERVER_URL";
const ETCD_ENDPOINT_ENV: &str = "TUGBOAT_TEST_ETCD_ENDPOINT";
const ETCD_IMAGE: &str = "quay.io/coreos/etcd:v3.6.7";
const ADMIN_NAMESPACE: &str = "rbac-system";
const ADMIN_SERVICE_ACCOUNT: &str = "integration-admin";
const ADMIN_SECRET: &str = "integration-admin-token";
const RBAC_API_GROUP: &str = "authorization";
const CLUSTER_ROLE_KIND: &str = "ClusterRole";
const SERVICE_ACCOUNT_SUBJECT_KIND: &str = "ServiceAccount";
const SERVICE_ACCOUNT_NAME_ANNOTATION: &str = "tugboat.io/service-account.name";
const SERVICE_ACCOUNT_TOKEN_SECRET_TYPE: &str = "tugboat.io/service-account-token";
const TOKEN_DATA_KEY: &str = "token";

pub struct TestContext {
    pub base_url: String,
    pub client: TugboatClient,
    pub admin_token: Option<String>,
    ca_cert_pem: Option<Vec<u8>>,
    ca_cert_path: Option<PathBuf>,
    masters_identity_pem: Option<Vec<u8>>,
    _guard: TestGuard,
}

pub struct SchedulerGuard {
    task: JoinHandle<()>,
    config_path: PathBuf,
}

pub struct ControllerManagerGuard {
    task: JoinHandle<()>,
    config_path: PathBuf,
}

enum TestGuard {
    External,
    Managed {
        apiserver: JoinHandle<()>,
        etcd: EtcdGuard,
        temp_paths: Vec<PathBuf>,
    },
}

enum EtcdGuard {
    Docker { container_name: String },
    External,
}

#[derive(Clone, Copy)]
enum AuthorizationModeSetting {
    AlwaysAllow,
    Rbac,
}

#[derive(Clone, Copy)]
enum TlsMode {
    ServerOnly,
    Mtls,
}

#[derive(Clone, Copy)]
struct SetupOptions {
    authorization_mode: AuthorizationModeSetting,
    seed_admin_token: bool,
    tls_mode: TlsMode,
}

struct GeneratedTlsAssets {
    server_cert_path: PathBuf,
    server_key_path: PathBuf,
    ca_cert_path: PathBuf,
    ca_cert_pem: Vec<u8>,
    masters_identity_pem: Option<Vec<u8>>,
    temp_paths: Vec<PathBuf>,
}

impl TestContext {
    pub async fn setup() -> Result<Option<Self>, DynError> {
        Self::setup_with_options(SetupOptions {
            authorization_mode: AuthorizationModeSetting::AlwaysAllow,
            seed_admin_token: false,
            tls_mode: TlsMode::ServerOnly,
        })
        .await
    }

    pub async fn setup_rbac() -> Result<Option<Self>, DynError> {
        Self::setup_with_options(SetupOptions {
            authorization_mode: AuthorizationModeSetting::Rbac,
            seed_admin_token: true,
            tls_mode: TlsMode::ServerOnly,
        })
        .await
    }

    pub async fn setup_rbac_with_mtls() -> Result<Option<Self>, DynError> {
        Self::setup_with_options(SetupOptions {
            authorization_mode: AuthorizationModeSetting::Rbac,
            seed_admin_token: true,
            tls_mode: TlsMode::Mtls,
        })
        .await
    }

    async fn setup_with_options(options: SetupOptions) -> Result<Option<Self>, DynError> {
        init_test_tracing();

        if matches!(
            options.authorization_mode,
            AuthorizationModeSetting::AlwaysAllow
        ) && matches!(options.tls_mode, TlsMode::ServerOnly)
            && let Some(base_url) = std::env::var_os(APISERVER_URL_ENV)
        {
            let base_url = base_url.to_string_lossy().into_owned();
            wait_for_healthz(&base_url, None, None).await?;
            return Ok(Some(Self {
                client: TugboatClient::new(base_url.clone()),
                base_url,
                admin_token: None,
                ca_cert_pem: None,
                ca_cert_path: None,
                masters_identity_pem: None,
                _guard: TestGuard::External,
            }));
        }

        let etcd = match std::env::var_os(ETCD_ENDPOINT_ENV) {
            Some(endpoint) => (endpoint.to_string_lossy().into_owned(), EtcdGuard::External),
            None => match start_etcd_container().await? {
                Some(started) => started,
                None => return Ok(None),
            },
        };

        let admin_token = if options.seed_admin_token {
            Some(seed_admin_service_account(&etcd.0).await?)
        } else {
            None
        };
        let tls_assets = generate_tls_assets(options.tls_mode)?;

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let base_url = format!("https://127.0.0.1:{port}");

        let mut temp_paths = Vec::new();
        let config_path = write_apiserver_config(port, &etcd.0, options, Some(&tls_assets))?;
        temp_paths.push(config_path.clone());
        let server =
            ApiServer::from_config(ApiServerConfig::load_from_file_or_panic(config_path)).await?;

        temp_paths.extend(tls_assets.temp_paths.clone());

        // Drop the pre-bound listener so the server can bind to the same port with TLS.
        drop(listener);
        let apiserver = tokio::spawn(server.run());
        let healthz_identity = tls_assets.masters_identity_pem.as_deref();
        if let Err(err) =
            wait_for_healthz(&base_url, healthz_identity, Some(&tls_assets.ca_cert_pem)).await
        {
            apiserver.abort();
            let _ = apiserver.await;
            return Err(err);
        }

        Ok(Some(Self {
            client: TugboatClient::new(base_url.clone()),
            base_url,
            admin_token,
            ca_cert_pem: Some(tls_assets.ca_cert_pem.clone()),
            ca_cert_path: Some(tls_assets.ca_cert_path.clone()),
            masters_identity_pem: tls_assets.masters_identity_pem,
            _guard: TestGuard::Managed {
                apiserver,
                etcd: etcd.1,
                temp_paths,
            },
        }))
    }

    pub fn http_client(&self) -> Result<reqwest::Client, DynError> {
        self.client_builder()?.build().map_err(Into::into)
    }

    pub fn bearer_client(&self, token: &str) -> Result<reqwest::Client, DynError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))?,
        );
        self.client_builder()?
            .default_headers(headers)
            .build()
            .map_err(Into::into)
    }

    pub fn admin_client(&self) -> Result<reqwest::Client, DynError> {
        let token = self
            .admin_token
            .as_deref()
            .ok_or("admin bearer token is not available in this test context")?;
        self.bearer_client(token)
    }

    pub fn masters_client(&self) -> Result<reqwest::Client, DynError> {
        let identity = self
            .masters_identity_pem
            .as_ref()
            .ok_or("mTLS identity is not available in this test context")?;
        self.client_builder()?
            .identity(reqwest::Identity::from_pem(identity)?)
            .build()
            .map_err(Into::into)
    }

    pub fn ca_cert_pem(&self) -> Option<&[u8]> {
        self.ca_cert_pem.as_deref()
    }

    fn client_builder(&self) -> Result<reqwest::ClientBuilder, DynError> {
        let mut builder = reqwest::Client::builder();
        if let Some(ca_pem) = &self.ca_cert_pem {
            builder = builder.add_root_certificate(reqwest::Certificate::from_pem(ca_pem)?);
        }
        Ok(builder)
    }

    pub fn start_scheduler(&self) -> Result<SchedulerGuard, DynError> {
        let config_path = std::env::temp_dir().join(format!(
            "tugboat-scheduler-it-{}-{}.toml",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let tls_section = self
            .ca_cert_path
            .as_ref()
            .map(|p| format!("\n[apiserver.tls]\nca_cert_path = \"{}\"\n", p.display()))
            .unwrap_or_default();
        std::fs::write(
            &config_path,
            format!(
                "[apiserver]\nurl = \"{}\"\n{tls_section}\n[scheduler]\nname = \"default-scheduler\"\nlease_duration_seconds = 15\nrenew_interval_seconds = 1\nscheduling_interval_seconds = 1\n",
                self.base_url
            ),
        )?;

        let task = tokio::spawn({
            let config_path = config_path.clone();
            async move {
                tugboat_scheduler::run_with_config_file(&config_path).await;
            }
        });

        Ok(SchedulerGuard { task, config_path })
    }

    pub async fn start_controller_manager(&self) -> Result<ControllerManagerGuard, DynError> {
        let config_path = std::env::temp_dir().join(format!(
            "tugboat-controller-manager-it-{}-{}.toml",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let tls_section = self
            .ca_cert_path
            .as_ref()
            .map(|p| format!("\n[apiserver.tls]\nca_cert_path = \"{}\"\n", p.display()))
            .unwrap_or_default();
        std::fs::write(
            &config_path,
            format!(
                "[apiserver]\nurl = \"{}\"\n{tls_section}\n[csi]\nrequeue_interval_seconds = 1\n\n[network]\nrequeue_interval_seconds = 1\n",
                self.base_url
            ),
        )?;

        let task = tokio::spawn({
            let config_path = config_path.clone();
            async move {
                tugboat_controller_manager::run_with_config_file(&config_path).await;
            }
        });

        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(ControllerManagerGuard { task, config_path })
    }
}

fn init_test_tracing() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
    });
}

impl Drop for SchedulerGuard {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.config_path);
    }
}

impl Drop for ControllerManagerGuard {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.config_path);
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        match self {
            Self::External => {}
            Self::Managed {
                apiserver,
                etcd,
                temp_paths,
            } => {
                apiserver.abort();
                match etcd {
                    EtcdGuard::Docker { container_name } => {
                        let _ = std::process::Command::new("docker")
                            .args(["rm", "-f", container_name])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                    }
                    EtcdGuard::External => {}
                }
                for path in temp_paths {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}

async fn wait_for_healthz(
    base_url: &str,
    identity_pem: Option<&[u8]>,
    ca_cert_pem: Option<&[u8]>,
) -> Result<(), DynError> {
    let mut builder = reqwest::Client::builder();
    if let Some(ca_pem) = ca_cert_pem {
        builder = builder.add_root_certificate(reqwest::Certificate::from_pem(ca_pem)?);
    }
    if let Some(identity_pem) = identity_pem {
        builder = builder.identity(reqwest::Identity::from_pem(identity_pem)?);
    }
    let client = builder.build()?;
    let url = format!("{base_url}/healthz");
    let mut last_error = None;
    for _ in 0..50 {
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                last_error = Some(format!("healthz returned {}", response.status()).into());
            }
            Err(err) => {
                last_error = Some(Box::new(err) as DynError);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(last_error.unwrap_or_else(|| "healthz did not become ready".into()))
}

async fn start_etcd_container() -> Result<Option<(String, EtcdGuard)>, DynError> {
    let container_name = format!(
        "tugboat-it-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
    );
    let output = match Command::new("docker")
        .args([
            "run",
            "--detach",
            "--rm",
            "--publish",
            "127.0.0.1::2379",
            "--name",
            &container_name,
            ETCD_IMAGE,
            "/usr/local/bin/etcd",
            "--advertise-client-urls",
            "http://0.0.0.0:2379",
            "--listen-client-urls",
            "http://0.0.0.0:2379",
        ])
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Box::new(err)),
    };
    if !output.status.success() {
        return Ok(None);
    }

    let port_output = Command::new("docker")
        .args(["port", &container_name, "2379/tcp"])
        .output()
        .await?;
    if !port_output.status.success() {
        let _ = Command::new("docker")
            .args(["rm", "-f", &container_name])
            .output()
            .await;
        return Ok(None);
    }

    let stdout = String::from_utf8(port_output.stdout)?;
    let endpoint = parse_docker_port(&stdout)
        .map(|port| format!("http://127.0.0.1:{port}"))
        .ok_or_else(|| format!("failed to parse docker port output: {stdout:?}"))?;

    wait_for_etcd(&endpoint).await?;
    Ok(Some((endpoint, EtcdGuard::Docker { container_name })))
}

fn parse_docker_port(output: &str) -> Option<u16> {
    output
        .lines()
        .find_map(|line| line.rsplit(':').next()?.trim().parse().ok())
}

async fn wait_for_etcd(endpoint: &str) -> Result<(), DynError> {
    let client = reqwest::Client::new();
    let url = format!("{endpoint}/health");
    let mut last_error = None;
    for _ in 0..50 {
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                last_error = Some(format!("etcd health returned {}", response.status()).into());
            }
            Err(err) => {
                last_error = Some(Box::new(err) as DynError);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(last_error.unwrap_or_else(|| "etcd did not become ready".into()))
}

async fn seed_admin_service_account(etcd_endpoint: &str) -> Result<String, DynError> {
    let store = ResourceStore::new(&[etcd_endpoint.to_string()]).await?;
    let token = format!(
        "rbac-admin-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );

    let _ = store
        .put_if_not_exists(Namespace {
            type_meta: Some(Namespace::type_meta()),
            object_meta: Some(ObjectMeta {
                name: Some(ADMIN_NAMESPACE.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await?;
    let _ = store
        .put_if_not_exists(ServiceAccount {
            type_meta: Some(ServiceAccount::type_meta()),
            object_meta: Some(ObjectMeta {
                name: Some(ADMIN_SERVICE_ACCOUNT.to_string()),
                namespace: Some(ADMIN_NAMESPACE.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await?;
    let _ = store
        .put_if_not_exists(Secret {
            type_meta: Some(Secret::type_meta()),
            object_meta: Some(ObjectMeta {
                name: Some(ADMIN_SECRET.to_string()),
                namespace: Some(ADMIN_NAMESPACE.to_string()),
                annotations: HashMap::from([(
                    SERVICE_ACCOUNT_NAME_ANNOTATION.to_string(),
                    ADMIN_SERVICE_ACCOUNT.to_string(),
                )]),
                ..Default::default()
            }),
            data: HashMap::from([(TOKEN_DATA_KEY.to_string(), BASE64_STANDARD.encode(&token))]),
            r#type: SERVICE_ACCOUNT_TOKEN_SECRET_TYPE.to_string(),
            ..Default::default()
        })
        .await?;
    let _ = store
        .put_if_not_exists(ClusterRoleBinding {
            type_meta: Some(ClusterRoleBinding::type_meta()),
            object_meta: Some(ObjectMeta {
                name: Some("integration-admin".to_string()),
                ..Default::default()
            }),
            subjects: vec![Subject {
                kind: SERVICE_ACCOUNT_SUBJECT_KIND.to_string(),
                api_group: RBAC_API_GROUP.to_string(),
                name: ADMIN_SERVICE_ACCOUNT.to_string(),
                namespace: Some(ADMIN_NAMESPACE.to_string()),
            }],
            role_ref: Some(RoleRef {
                api_group: RBAC_API_GROUP.to_string(),
                kind: CLUSTER_ROLE_KIND.to_string(),
                name: "cluster-admin".to_string(),
            }),
        })
        .await?;

    Ok(token)
}

fn write_apiserver_config(
    port: u16,
    etcd_endpoint: &str,
    options: SetupOptions,
    tls_assets: Option<&GeneratedTlsAssets>,
) -> Result<PathBuf, DynError> {
    let mut config = format!(
        "[http]\nlisten = \"127.0.0.1:{port}\"\n\n[etcd]\nendpoints = [\"{etcd_endpoint}\"]\n\n[authorization]\nmode = \"{}\"\n",
        match options.authorization_mode {
            AuthorizationModeSetting::AlwaysAllow => "AlwaysAllow",
            AuthorizationModeSetting::Rbac => "RBAC",
        }
    );
    if let Some(tls_assets) = tls_assets {
        if matches!(options.tls_mode, TlsMode::Mtls) {
            config.push_str(&format!(
                "\n[http.tls]\ncert_file = \"{}\"\nkey_file = \"{}\"\nclient_cert_file = \"{}\"\n",
                tls_assets.server_cert_path.display(),
                tls_assets.server_key_path.display(),
                tls_assets.ca_cert_path.display(),
            ));
        } else {
            config.push_str(&format!(
                "\n[http.tls]\ncert_file = \"{}\"\nkey_file = \"{}\"\n",
                tls_assets.server_cert_path.display(),
                tls_assets.server_key_path.display(),
            ));
        }
    }

    let path = std::env::temp_dir().join(format!(
        "tugboat-apiserver-it-{}-{}.toml",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::write(&path, config)?;
    Ok(path)
}

fn generate_tls_assets(tls_mode: TlsMode) -> Result<GeneratedTlsAssets, DynError> {
    let ca_key = generate_private_key()?;
    let ca_cert = build_ca_certificate(&ca_key)?;
    let ca_cert_pem = ca_cert.to_pem()?;
    let server_key = generate_private_key()?;
    let server_cert = build_signed_certificate(&ca_cert, &ca_key, &server_key, "127.0.0.1", false)?;

    let server_cert_path = write_temp_file("tugboat-it-server-cert", &server_cert.to_pem()?)?;
    let server_key_path = write_temp_file(
        "tugboat-it-server-key",
        &server_key.private_key_to_pem_pkcs8()?,
    )?;
    let ca_cert_path = write_temp_file("tugboat-it-ca-cert", &ca_cert_pem)?;

    let (masters_identity_pem, extra_paths) = if matches!(tls_mode, TlsMode::Mtls) {
        let client_key = generate_private_key()?;
        let client_cert =
            build_signed_certificate(&ca_cert, &ca_key, &client_key, "masters-user", true)?;
        let mut identity_pem = client_cert.to_pem()?;
        identity_pem.extend(client_key.private_key_to_pem_pkcs8()?);
        (Some(identity_pem), vec![])
    } else {
        (None, vec![])
    };

    let mut temp_paths = vec![
        server_cert_path.clone(),
        server_key_path.clone(),
        ca_cert_path.clone(),
    ];
    temp_paths.extend(extra_paths);

    Ok(GeneratedTlsAssets {
        server_cert_path,
        server_key_path,
        ca_cert_path,
        ca_cert_pem,
        masters_identity_pem,
        temp_paths,
    })
}

fn generate_private_key() -> Result<PKey<Private>, DynError> {
    Ok(PKey::from_rsa(Rsa::generate(2048)?)?)
}

fn build_ca_certificate(key: &PKey<Private>) -> Result<X509, DynError> {
    let mut name = X509NameBuilder::new()?;
    name.append_entry_by_text("CN", "tugboat-it-ca")?;
    let name = name.build();

    let mut builder = X509::builder()?;
    builder.set_version(2)?;
    builder.set_subject_name(&name)?;
    builder.set_issuer_name(&name)?;
    builder.set_pubkey(key)?;
    builder.set_not_before(Asn1Time::days_from_now(0)?.as_ref())?;
    builder.set_not_after(Asn1Time::days_from_now(30)?.as_ref())?;
    builder.append_extension(BasicConstraints::new().critical().ca().build()?)?;
    builder.append_extension(
        KeyUsage::new()
            .critical()
            .key_cert_sign()
            .crl_sign()
            .build()?,
    )?;
    builder.sign(key, MessageDigest::sha256())?;
    Ok(builder.build())
}

fn build_signed_certificate(
    ca_cert: &X509,
    ca_key: &PKey<Private>,
    key: &PKey<Private>,
    common_name: &str,
    client_auth: bool,
) -> Result<X509, DynError> {
    let mut name = X509NameBuilder::new()?;
    name.append_entry_by_text("CN", common_name)?;
    if client_auth {
        name.append_entry_by_text("O", "system:masters")?;
    }
    let name = name.build();

    let mut builder = X509::builder()?;
    builder.set_version(2)?;
    builder.set_subject_name(&name)?;
    builder.set_issuer_name(ca_cert.subject_name())?;
    builder.set_pubkey(key)?;
    builder.set_not_before(Asn1Time::days_from_now(0)?.as_ref())?;
    builder.set_not_after(Asn1Time::days_from_now(30)?.as_ref())?;
    builder.append_extension(BasicConstraints::new().build()?)?;
    if client_auth {
        builder.append_extension(ExtendedKeyUsage::new().client_auth().build()?)?;
    } else {
        builder.append_extension(ExtendedKeyUsage::new().server_auth().build()?)?;
        // Add IP SAN so rustls can verify the server certificate against the IP address.
        let san = SubjectAlternativeName::new()
            .ip(common_name)
            .build(&builder.x509v3_context(Some(ca_cert), None))?;
        builder.append_extension(san)?;
    }
    builder.append_extension(
        KeyUsage::new()
            .digital_signature()
            .key_encipherment()
            .build()?,
    )?;
    builder.sign(ca_key, MessageDigest::sha256())?;
    Ok(builder.build())
}

fn write_temp_file(prefix: &str, bytes: &[u8]) -> Result<PathBuf, DynError> {
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}.pem",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::write(&path, bytes)?;
    Ok(path)
}
