use serde::Deserialize;
use utoipa::ToSchema;

mod utils;
mod v1_core;

#[derive(Deserialize, ToSchema)]
struct NamespacedPathParams {
    namespace: String,
}
