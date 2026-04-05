use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
struct Args {
    #[arg(
        long,
        help = "Path to the tugboat-controller-manager config file",
        default_value = "/etc/tugboat/controller-manager/config.toml"
    )]
    config: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    tugboat_controller_manager::run_with_config_file(&args.config).await;
}
