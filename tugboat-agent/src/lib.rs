use clap::Parser;

mod config;
mod reconciler;
mod runtime;

#[derive(Debug, Parser)]
struct Args {
    #[arg(
        long,
        help = "Path to the tugboat-agent config file",
        default_value = "/etc/tugboat/agent/config.toml"
    )]
    config: String,
}

pub async fn run() {
    let args = Args::parse();
    let config = config::AgentConfig::load_or_panic(args.config);
}
