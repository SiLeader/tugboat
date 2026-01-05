use crate::auth::load_auth_or_anonymous;
use crate::{Arch, Error, Format, VmImageRegistry};
use oci_distribution::Reference;
use oci_distribution::client::{Config, ImageLayer};
use serde::Serialize;
use tracing::debug;

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
        let metadata = TugboatImageMetadata {
            format: match format {
                Format::Qcow2 => "qcow2".to_string(),
            },
            arch: match arch {
                Arch::X86_64 => "x86_64".to_string(),
            },
        };
        let metadata = serde_json::to_vec(&metadata)?;

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

        let reference: Reference = tag.parse()?;
        let auth = load_auth_or_anonymous(&reference.registry());

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
