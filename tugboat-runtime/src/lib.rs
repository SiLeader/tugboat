use clap::Parser;
mod vm;

#[derive(Parser, Debug, Clone)]
pub struct RuntimeArgs {}

pub async fn execute(args: RuntimeArgs) {}
