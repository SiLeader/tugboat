use crate::data::StatusResponse;
use crate::endpoints::selector::Selector;
use crate::endpoints::v1_core::register_v1_core;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_actix_web::service_config::ServiceConfig;

mod selector;
mod utils;
mod v1_core;
mod watch_utils;

pub(super) fn register_endpoints(config: &mut ServiceConfig) {
    register_v1_core(config);
}

#[derive(Deserialize, ToSchema)]
struct NamespacedPathParams {
    namespace: String,
}

#[derive(Deserialize, Copy, Clone)]
#[serde(rename_all = "camelCase")]
enum WatchOption {
    True, // default watch mode
    NdJson,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    watch: Option<WatchOption>,
    resource_version: Option<String>,
    field_selector: Option<String>,
    label_selector: Option<String>,
}

impl ListQuery {
    fn to_field_selector(&self) -> Result<Option<Vec<Selector>>, StatusResponse> {
        if let Some(field_selector) = &self.field_selector {
            Selector::try_parse(&field_selector).map(Some)
        } else {
            Ok(None)
        }
    }

    fn to_label_selector(&self) -> Result<Option<Vec<Selector>>, StatusResponse> {
        if let Some(label_selector) = &self.label_selector {
            Selector::try_parse(&label_selector).map(Some)
        } else {
            Ok(None)
        }
    }
}
