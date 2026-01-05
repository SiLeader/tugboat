// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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
