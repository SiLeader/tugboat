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

use crate::data::StatusResponse;
use actix_web::body::BoxBody;
use actix_web::{HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use tugboat_resources::manifests::meta::v1::{ObjectMeta, TypeMeta};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ResourceList {
    #[serde(flatten)]
    type_meta: TypeMeta,
    #[serde(rename = "metadata")]
    object_meta: ObjectMeta,
    items: Vec<serde_json::Value>,
}

impl ResourceList {
    pub(crate) fn from_serializable<T: Serialize>(items: Vec<T>) -> Result<Self, Box<StatusResponse>> {
        match items
            .into_iter()
            .map(|v| serde_json::to_value(v))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(items) => Ok(Self::from_raw_items(items)),
            Err(_e) => Err(Box::new(StatusResponse::internal_error(
                "Failed to serialize data",
                None,
            ))),
        }
    }

    pub(crate) fn from_raw_items(items: Vec<serde_json::Value>) -> Self {
        Self {
            type_meta: TypeMeta {
                api_version: Some("v1".to_string()),
                kind: Some("List".to_string()),
            },
            object_meta: ObjectMeta::default(),
            items,
        }
    }
}

impl Responder for ResourceList {
    type Body = BoxBody;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        self.into()
    }
}

impl From<ResourceList> for HttpResponse {
    fn from(value: ResourceList) -> Self {
        HttpResponse::Ok().json(value)
    }
}
