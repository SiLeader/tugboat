use clap::{Parser, Subcommand};
use tracing::debug;
use tracing_subscriber::EnvFilter;

mod build;

#[derive(Debug, Parser)]
struct Args {
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(Debug, Subcommand)]
enum SubCommand {
    Build(build::BuildArgs),
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    debug!("Command line arguments: {args:?}");

    match args.subcommand {
        SubCommand::Build(build_args) => build::run_build(build_args).await,
    }
}
