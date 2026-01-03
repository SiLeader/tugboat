use crate::data::{ReadResponse, StatusResponse};
use crate::operator::ApiOperator;
use actix_web::get;
use actix_web::web::{Data, Path};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::ShipClass;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub(super) struct ReadParams {
    name: String,
}

#[utoipa::path()]
#[get("/v1/shipclasses/{name}")]
pub(super) async fn handle_shipclass_read(
    path: Path<ReadParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<ShipClass>, StatusResponse> {
    let shipclass = operator.store.get(None, &path.name).await?;

    match shipclass {
        Some(data) => Ok(ReadResponse::new(data.apply_revision())),
        None => Err(StatusResponse::not_found(
            "ShipClass not found",
            Some(serde_json::json!({ "name": path.name })),
        )),
    }
}
