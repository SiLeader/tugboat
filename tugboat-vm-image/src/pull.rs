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
use crate::compress::decompress_gzip_to_writer;
use crate::{
    Error, Format, MAX_DISK_IMAGE_UNCOMPRESSED_BYTES, METADATA_MEDIA_TYPE, VmImageRegistry,
};
use oci_distribution::Reference;
use oci_distribution::client::ImageLayer;
use serde::Serialize;
use std::fs::File;
use std::io::Write;
use tracing::{debug, info};

pub struct Image {
    pub location: String,
    pub format: Format,
}

#[derive(Serialize)]
struct CacheMetadata<'a> {
    image: &'a str,
    format: &'a str,
    disk: &'a str,
}

impl VmImageRegistry {
    pub async fn pull(&self, image: &str, insecure: Option<bool>) -> Result<Image, Error> {
        let reference: Reference = image.parse()?;

        let auth = load_auth_or_anonymous(reference.registry()).await;

        let client = self.get_client(reference.registry(), insecure);
        let data = client
            .pull(
                &reference,
                &auth,
                vec![
                    Format::Qcow2.media_type(),
                    Format::Raw.media_type(),
                    METADATA_MEDIA_TYPE,
                ],
            )
            .await?;

        let (layer, format) = select_disk_layer(image, data.layers)?;
        let location = self.save_disk_layer(image, &layer, format).await?;
        info!("Image '{image}' pull finished");
        Ok(Image { location, format })
    }

    async fn save_disk_layer(
        &self,
        image: &str,
        layer: &ImageLayer,
        format: Format,
    ) -> Result<String, Error> {
        let dir = self.directory.join(layer.sha256_digest());
        tokio::fs::create_dir_all(&dir).await?;
        let filename = dir.join(format.file_name());
        let reference_path = dir.join("reference");
        let metadata_path = dir.join("metadata.json");

        debug!("Saving {} disk layer to {filename:?}", format.as_str());
        let location = filename
            .to_str()
            .ok_or(Error::FileLocationEncode)?
            .to_string();
        let mut file = File::create(&filename)?;
        if let Err(err) = decompress_gzip_to_writer(
            layer.data.as_slice(),
            &mut file,
            MAX_DISK_IMAGE_UNCOMPRESSED_BYTES,
        ) {
            let _ = std::fs::remove_file(&filename);
            return Err(err);
        }
        file.flush()?;
        file.sync_all()?;

        tokio::fs::write(reference_path, image).await?;
        let metadata = CacheMetadata {
            image,
            format: format.as_str(),
            disk: format.file_name(),
        };
        tokio::fs::write(metadata_path, serde_json::to_vec_pretty(&metadata)?).await?;
        Ok(location)
    }
}

fn select_disk_layer(image: &str, layers: Vec<ImageLayer>) -> Result<(ImageLayer, Format), Error> {
    let mut selected = None;
    for layer in layers {
        let Some(format) = format_for_media_type(&layer.media_type) else {
            continue;
        };
        if selected.is_some() {
            return Err(Error::MultipleDiskImageLayers(image.to_string()));
        }
        selected = Some((layer, format));
    }
    selected.ok_or_else(|| Error::DiskImageMissing(image.to_string()))
}

fn format_for_media_type(media_type: &str) -> Option<Format> {
    [Format::Qcow2, Format::Raw]
        .into_iter()
        .find(|format| media_type == format.media_type())
}

#[cfg(test)]
mod tests {
    use super::select_disk_layer;
    use crate::compress::compress_gzip;
    use crate::{Error, Format, METADATA_MEDIA_TYPE, VmImageRegistry};
    use oci_distribution::client::ImageLayer;

    fn layer(media_type: &str, data: &[u8]) -> ImageLayer {
        ImageLayer {
            media_type: media_type.to_string(),
            data: data.to_vec(),
            annotations: None,
        }
    }

    #[test]
    fn select_disk_layer_accepts_raw_media_type() {
        let (layer, format) = select_disk_layer(
            "registry.example/app:raw",
            vec![
                layer(METADATA_MEDIA_TYPE, b"{}"),
                layer(Format::Raw.media_type(), b"raw"),
            ],
        )
        .expect("raw layer");

        assert_eq!(format, Format::Raw);
        assert_eq!(layer.data, b"raw");
    }

    #[test]
    fn select_disk_layer_accepts_qcow2_media_type() {
        let (_layer, format) = select_disk_layer(
            "registry.example/app:qcow2",
            vec![layer(Format::Qcow2.media_type(), b"qcow2")],
        )
        .expect("qcow2 layer");

        assert_eq!(format, Format::Qcow2);
    }

    #[test]
    fn select_disk_layer_rejects_missing_supported_disk_layer() {
        let err = match select_disk_layer(
            "registry.example/app:missing",
            vec![layer(METADATA_MEDIA_TYPE, b"{}")],
        ) {
            Ok(_) => panic!("missing disk layer should fail"),
            Err(err) => err,
        };

        assert!(
            matches!(err, Error::DiskImageMissing(image) if image == "registry.example/app:missing")
        );
    }

    #[test]
    fn select_disk_layer_rejects_multiple_supported_disk_layers() {
        let err = match select_disk_layer(
            "registry.example/app:multi",
            vec![
                layer(Format::Qcow2.media_type(), b"qcow2"),
                layer(Format::Raw.media_type(), b"raw"),
            ],
        ) {
            Ok(_) => panic!("multiple disk layers should fail"),
            Err(err) => err,
        };

        assert!(
            matches!(err, Error::MultipleDiskImageLayers(image) if image == "registry.example/app:multi")
        );
    }

    #[tokio::test]
    async fn save_disk_layer_stores_raw_disk_and_cache_metadata() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let registry = VmImageRegistry::new(temp_dir.path());
        let compressed = compress_gzip(b"raw-disk").expect("compress");
        let layer = layer(Format::Raw.media_type(), &compressed);

        let location = registry
            .save_disk_layer("registry.example/app:raw", &layer, Format::Raw)
            .await
            .expect("save raw layer");

        let disk = std::path::PathBuf::from(&location);
        assert_eq!(
            disk.file_name().and_then(|name| name.to_str()),
            Some("disk.raw")
        );
        assert_eq!(std::fs::read(&disk).expect("disk"), b"raw-disk");

        let dir = disk.parent().expect("cache dir");
        assert_eq!(
            std::fs::read_to_string(dir.join("reference")).expect("reference"),
            "registry.example/app:raw"
        );

        let metadata: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.join("metadata.json")).expect("metadata file"),
        )
        .expect("metadata json");
        assert_eq!(metadata["image"], "registry.example/app:raw");
        assert_eq!(metadata["format"], "raw");
        assert_eq!(metadata["disk"], "disk.raw");
    }
}
