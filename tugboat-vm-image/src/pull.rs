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
