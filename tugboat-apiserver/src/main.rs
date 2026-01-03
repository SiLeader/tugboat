use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tugboat_apiserver::ApiServer;
use tugboat_apiserver::config::ApiServerConfig;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value = "/etc/tugboat/apiserver/config.toml")]
    config: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    info!("Starting Tugboat API server");

    let config = ApiServerConfig::load_from_file_or_panic(args.config);
    ApiServer::from_config(config).await.run().await;
}
