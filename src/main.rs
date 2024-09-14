use clap::{Parser, Subcommand};
use tugboat_runtime::RuntimeArgs;

#[derive(Parser, Debug, Clone)]
struct Args {
    #[clap(subcommand)]
    sub: SubCommand,
}

#[derive(Debug, Clone, Subcommand)]
enum SubCommand {
    Runtime(RuntimeArgs),
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    match args.sub {
        SubCommand::Runtime(args) => tugboat_runtime::execute(args).await,
    }
}
