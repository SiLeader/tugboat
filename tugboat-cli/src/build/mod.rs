mod auth;
mod error;
mod parser;

use crate::build::parser::Imagefile;
use clap::Parser;
use oci_distribution::client::{ClientConfig, ClientProtocol, Config, ImageLayer};
use oci_distribution::{Client, Reference};
use std::str::FromStr;
use tracing::{debug, info};

#[derive(Debug, Parser)]
pub(crate) struct BuildArgs {
    #[arg(long, short, help = "Path to Imagefile", default_value = "Imagefile")]
    file: String,

    #[arg(long, short, help = "OCI Artifact Tags", required = true)]
    tag: String,

    #[arg(long, help = "Allow insecure registry connections")]
    insecure: bool,

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

    let metadata = imagefile.metadata().expect("Failed to generate metadata");

    let layers = vec![
        ImageLayer {
            media_type: "application/vnd.tugboat.disk.qcow2.v1+gzip".to_string(),
            data: disk_data,
            annotations: None,
        },
        ImageLayer {
            media_type: "application/vnd.tugboat.metadata.v1+json".to_string(),
            data: metadata,
            annotations: None,
        },
    ];

    let reference: Reference = args.tag.parse().expect("Failed to parse tag");
    let auth = auth::load_auth_or_anonymous(&reference.registry());

    let client = Client::new(ClientConfig {
        protocol: if args.insecure {
            ClientProtocol::Http
        } else {
            ClientProtocol::Https
        },
        ..Default::default()
    });
    debug!("Pushing image to {}", reference);
    client
        .push(
            &reference,
            &layers,
            Config::oci_v1(b"{}".to_vec(), None),
            &auth,
            None,
        )
        .await
        .expect("Failed to push image");
}
