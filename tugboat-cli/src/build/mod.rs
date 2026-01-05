mod error;
mod parser;

use crate::build::parser::Imagefile;
use clap::Parser;
use std::str::FromStr;
use tracing::{debug, info};
use tugboat_vm_image::VmImageRegistry;

#[derive(Debug, Parser)]
pub(crate) struct BuildArgs {
    #[arg(long, short, help = "Path to Imagefile", default_value = "Imagefile")]
    file: String,

    #[arg(long, short, help = "OCI Artifact Tags", required = true)]
    tag: String,

    #[arg(help = "Context directory")]
    context: String,
}

pub(crate) async fn run_build(args: BuildArgs) {
    info!(
        "Build command: Imagefile {} Context {}",
        args.file, args.context
    );

    let imagefile = Imagefile::from_str(
        tokio::fs::read_to_string(args.file)
            .await
            .expect("Failed to read Imagefile")
            .as_str(),
    )
    .expect("Failed to parse Imagefile");
    debug!("Imagefile: {:?}", imagefile);

    let disk_data = imagefile
        .read_disk(&args.context)
        .await
        .expect("Failed to read context directory");
    debug!("Disk data size: {} bytes", disk_data.len());

    let client = VmImageRegistry::default();
    client
        .push(args.tag, imagefile.arch, imagefile.format, disk_data, None)
        .await
        .expect("Failed to push image");
}
