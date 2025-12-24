use clap::Parser;
use serde::Deserialize;
use std::process::exit;
use tracing_subscriber::EnvFilter;
use tugboat_runtime::{Error, QemuVmBuilder, QemuVmConfig, RuntimeArgs, execute};

#[derive(Parser)]
struct Args {
    #[arg(
        help = "Path to the config file",
        default_value = "/etc/tugboat/runtime/config.toml"
    )]
    config: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let config = {
        let file = std::fs::read_to_string(args.config).expect("Failed to read config file");
        let config: Config = toml::from_str(&file).expect("Failed to parse config file as TOML");
        config
    };
    let spawner = QemuVmBuilder::new(config.qemu);
    let args = handle_error(RuntimeArgs::from_env());

    handle_error(execute(spawner, args).await);
}

fn handle_error<T>(value: tugboat_runtime::Result<T>) -> T {
    match value {
        Ok(value) => value,
        Err(e) => match e {
            Error::Io(e) => {
                eprintln!("IO Error: {}", e);
                exit(1);
            }
            Error::Environment(name, e) => {
                eprintln!("Environment Error: {}: {}", name, e);
                exit(2);
            }
            Error::EnvironmentParseError(name, message) => {
                eprintln!("Environment Parse Error: {}: {}", name, message);
                exit(3);
            }
        },
    }
}

#[derive(Deserialize)]
struct Config {
    qemu: QemuVmConfig,
}
