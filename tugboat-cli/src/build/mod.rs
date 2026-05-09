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

use crate::build::error::BuildError;
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

    #[arg(long, help = "Use HTTP instead of HTTPS")]
    http: bool,

    #[arg(help = "Context directory")]
    context: String,
}

pub(crate) async fn run_build(args: BuildArgs) -> Result<(), BuildError> {
    info!(
        "Build command: Imagefile {} Context {}",
        args.file, args.context
    );

    let imagefile_content = tokio::fs::read_to_string(&args.file)
        .await
        .map_err(|source| BuildError::ReadImagefile {
            path: args.file.clone(),
            source,
        })?;
    let imagefile =
        Imagefile::from_str(&imagefile_content).map_err(|source| BuildError::ParseImagefile {
            path: args.file.clone(),
            source: Box::new(source),
        })?;
    debug!("Imagefile: {:?}", imagefile);

    let disk_path = imagefile.disk_path(&args.context);
    let disk_data =
        imagefile
            .read_disk(&args.context)
            .await
            .map_err(|source| BuildError::ReadDisk {
                path: disk_path,
                source,
            })?;
    debug!("Disk data size: {} bytes", disk_data.len());

    let client = VmImageRegistry::new("/tmp");
    let tag = args.tag.clone();
    client
        .push(
            args.tag,
            imagefile.arch,
            imagefile.format,
            disk_data,
            if args.http { Some(true) } else { None },
        )
        .await
        .map_err(|source| BuildError::PushImage { tag, source })?;
    Ok(())
}
