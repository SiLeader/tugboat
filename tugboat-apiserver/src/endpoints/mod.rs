use crate::endpoints::v1_core::register_v1_core;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_actix_web::service_config::ServiceConfig;

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

#[derive(Deserialize)]
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
}
