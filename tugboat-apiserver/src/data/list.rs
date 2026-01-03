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
    pub(crate) fn from_serializable<T: Serialize>(items: Vec<T>) -> Result<Self, StatusResponse> {
        match items
            .into_iter()
            .map(|v| serde_json::to_value(v))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(items) => Ok(Self::from_raw_items(items)),
            Err(_e) => Err(StatusResponse::internal_error(
                "Failed to serialize data",
                None,
            )),
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
    pub(crate) fn get_raw(&self, index: usize) -> Option<&serde_json::Value> {
        self.items.get(index)
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &serde_json::Value> {
        self.items.iter()
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
