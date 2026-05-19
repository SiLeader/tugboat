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

use crate::auth::load_auth_or_anonymous;
use crate::compress::compress_gzip;
use crate::{Arch, Error, Format, METADATA_MEDIA_TYPE, VmImageRegistry};
use oci_distribution::Reference;
use oci_distribution::client::{Config, ImageLayer};
use serde::Serialize;
use tracing::{debug, info};

#[derive(Serialize)]
struct TugboatImageMetadata {
    format: String,
    arch: String,
}

impl VmImageRegistry {
    pub async fn push(
        &self,
        tag: String,
        arch: Arch,
        format: Format,
        disk_data: Vec<u8>,
        insecure: Option<bool>,
    ) -> Result<(), Error> {
        info!("Pushing image: {tag} ({} bytes)", disk_data.len());
        let layers = build_layers(arch, format, disk_data)?;

        let reference: Reference = tag.parse()?;
        let auth = load_auth_or_anonymous(reference.registry()).await;

        let client = self.get_client(reference.registry(), insecure);
        debug!("Pushing image to {}", reference);
        client
            .push(
                &reference,
                &layers,
                Config::oci_v1(b"{}".to_vec(), None),
                &auth,
                None,
            )
            .await?;
        Ok(())
    }
}

fn build_layers(arch: Arch, format: Format, disk_data: Vec<u8>) -> Result<Vec<ImageLayer>, Error> {
    let metadata = TugboatImageMetadata {
        format: format.as_str().to_string(),
        arch: match arch {
            Arch::X64 => "x64".to_string(),
        },
    };
    let metadata = serde_json::to_vec(&metadata)?;
    let original_size = disk_data.len();
    // TODO: stream disk data through gzip into OCI push when the client supports it cleanly.
    let disk_data = compress_gzip(&disk_data)?;
    debug!(
        "Compressed disk data size: {} bytes ({}% compressed)",
        disk_data.len(),
        original_size
            .checked_sub(disk_data.len())
            .and_then(|saved| saved.checked_mul(100))
            .and_then(|saved| saved.checked_div(original_size))
            .unwrap_or(0)
    );

    Ok(vec![
        ImageLayer {
            media_type: format.media_type().to_string(),
            data: disk_data,
            annotations: None,
        },
        ImageLayer {
            media_type: METADATA_MEDIA_TYPE.to_string(),
            data: metadata,
            annotations: None,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::build_layers;
    use crate::{Arch, Format, METADATA_MEDIA_TYPE};

    #[test]
    fn build_layers_uses_raw_media_type_and_metadata_format() {
        let layers = build_layers(Arch::X64, Format::Raw, b"raw-disk".to_vec()).expect("layers");

        assert_eq!(layers[0].media_type, Format::Raw.media_type());
        assert_eq!(layers[1].media_type, METADATA_MEDIA_TYPE);

        let metadata: serde_json::Value =
            serde_json::from_slice(&layers[1].data).expect("metadata json");
        assert_eq!(metadata["format"], "raw");
        assert_eq!(metadata["arch"], "x64");
    }

    #[test]
    fn build_layers_keeps_qcow2_media_type_and_metadata_format() {
        let layers =
            build_layers(Arch::X64, Format::Qcow2, b"qcow2-disk".to_vec()).expect("layers");

        assert_eq!(layers[0].media_type, Format::Qcow2.media_type());
        assert_eq!(layers[1].media_type, METADATA_MEDIA_TYPE);

        let metadata: serde_json::Value =
            serde_json::from_slice(&layers[1].data).expect("metadata json");
        assert_eq!(metadata["format"], "qcow2");
        assert_eq!(metadata["arch"], "x64");
    }
}
