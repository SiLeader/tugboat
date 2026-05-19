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

use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum BuildError {
    #[error("Invalid Imagefile format: {0} (supported values: qcow2, raw)")]
    InvalidFormat(String),
    #[error("Invalid Imagefile arch: {0}")]
    InvalidArch(String),
    #[error("Imagefile must include a FROM line with a disk image path")]
    MissingFrom,
    #[error("failed to read Imagefile '{path}': {source}")]
    ReadImagefile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse Imagefile '{path}': {source}")]
    ParseImagefile {
        path: String,
        #[source]
        source: Box<BuildError>,
    },
    #[error("failed to read disk image '{path}' from build context: {source}")]
    ReadDisk {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to push image '{tag}': {source}")]
    PushImage {
        tag: String,
        #[source]
        source: tugboat_vm_image::Error,
    },
}
