use clap::Parser;
use tugboat_runtime::{execute, RuntimeArgs};

#[tokio::main]
async fn main() {
    let args = RuntimeArgs::parse();

    execute(args).await;
}
