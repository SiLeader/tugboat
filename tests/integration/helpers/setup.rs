use std::error::Error;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing_subscriber::EnvFilter;
use tugboat_apiserver::ApiServer;
use tugboat_apiserver::config::ApiServerConfig;
use tugboat_client::TugboatClient;

type DynError = Box<dyn Error + Send + Sync>;

const APISERVER_URL_ENV: &str = "TUGBOAT_TEST_APISERVER_URL";
const ETCD_ENDPOINT_ENV: &str = "TUGBOAT_TEST_ETCD_ENDPOINT";
const ETCD_IMAGE: &str = "quay.io/coreos/etcd:v3.6.7";

pub struct TestContext {
    pub base_url: String,
    pub client: TugboatClient,
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
    },
}

enum EtcdGuard {
    Docker { container_name: String },
    External,
}

impl TestContext {
    pub async fn setup() -> Result<Option<Self>, DynError> {
        init_test_tracing();

        if let Some(base_url) = std::env::var_os(APISERVER_URL_ENV) {
            let base_url = base_url.to_string_lossy().into_owned();
            wait_for_healthz(&base_url).await?;
            return Ok(Some(Self {
                client: TugboatClient::new(base_url.clone()),
                base_url,
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

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let base_url = format!("http://127.0.0.1:{port}");
        let server = ApiServer::from_config(ApiServerConfig::new(
            format!("127.0.0.1:{port}"),
            vec![etcd.0.clone()],
        ))
        .await?;
        let apiserver = tokio::spawn(server.run_with_listener(listener));
        if let Err(err) = wait_for_healthz(&base_url).await {
            apiserver.abort();
            let _ = apiserver.await;
            return Err(err);
        }

        Ok(Some(Self {
            client: TugboatClient::new(base_url.clone()),
            base_url,
            _guard: TestGuard::Managed {
                apiserver,
                etcd: etcd.1,
            },
        }))
    }

    pub fn start_scheduler(&self) -> Result<SchedulerGuard, DynError> {
        let config_path = std::env::temp_dir().join(format!(
            "tugboat-scheduler-it-{}-{}.toml",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        std::fs::write(
            &config_path,
            format!(
                "[apiserver]\nurl = \"{}\"\n\n[scheduler]\nname = \"default-scheduler\"\nlease_duration_seconds = 15\nrenew_interval_seconds = 1\nscheduling_interval_seconds = 1\n",
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
        std::fs::write(
            &config_path,
            format!(
                "[apiserver]\nurl = \"{}\"\n\n[csi]\nrequeue_interval_seconds = 1\n\n[network]\nrequeue_interval_seconds = 1\n",
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
            Self::Managed { apiserver, etcd } => {
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
            }
        }
    }
}

async fn wait_for_healthz(base_url: &str) -> Result<(), DynError> {
    let client = reqwest::Client::new();
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
