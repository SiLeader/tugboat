use crate::auth::load_auth_or_anonymous;
use crate::{Error, VmImageRegistry};
use oci_distribution::Reference;

pub struct Image {
    pub location: String,
}

impl VmImageRegistry {
    pub async fn pull(&self, image: String, insecure: Option<bool>) -> Result<Image, Error> {
        let reference: Reference = image.parse()?;

        let auth = load_auth_or_anonymous(reference.registry());

        let client = self.get_client(reference.registry(), insecure);
        let data = client
            .pull(
                &reference,
                &auth,
                vec![
                    "application/vnd.tugboat.disk.qcow2.v1+gzip",
                    // "application/vnd.tugboat.metadata.v1+json",
                ],
            )
            .await?;

        for layer in data.layers {
            let filename = if layer.media_type.contains("qcow2") {
                self.directory
                    .join(layer.sha256_digest())
                    .join("disk.qcow2")
            } else {
                continue;
            };
            let location = filename
                .to_str()
                .ok_or(Error::FileLocationEncode)?
                .to_string();
            tokio::fs::write(&filename, layer.data).await?;
            return Ok(Image { location });
        }
        Err(Error::DiskImageMissing(image))
    }
}
