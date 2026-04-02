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

use crate::csi::ResolvedCsiSecrets;
use crate::reconciler::ShipReconciler;
use crate::reconciler::error::{InvalidCsiSecretDataError, ReconcileError};
use crate::reconciler::volume::PersistentVolumeClaimVolumeInfo;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use std::collections::HashMap;
use tugboat_client::Api;
use tugboat_resources::manifests::core::v1::{Secret, SecretReference};

impl ShipReconciler {
    pub(crate) async fn resolve_csi_secrets(
        &self,
        volume: &PersistentVolumeClaimVolumeInfo,
    ) -> Result<ResolvedCsiSecrets, ReconcileError> {
        Ok(ResolvedCsiSecrets {
            controller_publish: self
                .load_secret_reference(
                    &volume.volume_name,
                    "controller_publish_secret_ref",
                    volume.source.controller_publish_secret_ref.as_ref(),
                )
                .await?,
            node_expand: self
                .load_secret_reference(
                    &volume.volume_name,
                    "node_expand_secret_ref",
                    volume.source.node_expand_secret_ref.as_ref(),
                )
                .await?,
            node_publish: self
                .load_secret_reference(
                    &volume.volume_name,
                    "node_publish_secret_ref",
                    volume.source.node_publish_secret_ref.as_ref(),
                )
                .await?,
            node_stage: self
                .load_secret_reference(
                    &volume.volume_name,
                    "node_stage_secret_ref",
                    volume.source.node_stage_secret_ref.as_ref(),
                )
                .await?,
            mount_flags: volume
                .source
                .mount_options
                .iter()
                .filter(|value| !value.is_empty())
                .cloned()
                .collect(),
        })
    }

    async fn load_secret_reference(
        &self,
        volume_name: &str,
        field: &str,
        reference: Option<&SecretReference>,
    ) -> Result<HashMap<String, String>, ReconcileError> {
        let Some(reference) = reference else {
            return Ok(Default::default());
        };
        if reference.name.is_empty() || reference.namespace.is_empty() {
            return Err(ReconcileError::InvalidCsiSecretReference {
                volume: volume_name.to_string(),
                field: field.to_string(),
            });
        }

        let api: Api<Secret> = Api::namespaced(self.client.clone(), &reference.namespace);
        let Some(secret) = api.get(&reference.name).await? else {
            return Err(ReconcileError::CsiSecretNotFound {
                volume: volume_name.to_string(),
                field: field.to_string(),
                namespace: reference.namespace.clone(),
                name: reference.name.clone(),
            });
        };

        decode_csi_secret_data(
            volume_name,
            field,
            &reference.namespace,
            &reference.name,
            secret,
        )
    }
}

pub(super) fn decode_csi_secret_data(
    volume_name: &str,
    field: &str,
    namespace: &str,
    secret_name: &str,
    secret: Secret,
) -> Result<HashMap<String, String>, ReconcileError> {
    let mut data = HashMap::new();
    for (key, value) in secret.data {
        let decoded = BASE64_STANDARD.decode(value).map_err(|err| {
            invalid_csi_secret_data(
                volume_name,
                field,
                namespace,
                secret_name,
                &key,
                err.to_string(),
            )
        })?;
        let decoded = String::from_utf8(decoded).map_err(|err| {
            invalid_csi_secret_data(
                volume_name,
                field,
                namespace,
                secret_name,
                &key,
                err.to_string(),
            )
        })?;
        data.insert(key, decoded);
    }
    data.extend(secret.string_data);
    Ok(data)
}

fn invalid_csi_secret_data(
    volume_name: &str,
    field: &str,
    namespace: &str,
    secret_name: &str,
    key: &str,
    reason: String,
) -> ReconcileError {
    ReconcileError::InvalidCsiSecretData(Box::new(InvalidCsiSecretDataError {
        volume: volume_name.to_string(),
        field: field.to_string(),
        namespace: namespace.to_string(),
        name: secret_name.to_string(),
        key: key.to_string(),
        reason,
    }))
}
