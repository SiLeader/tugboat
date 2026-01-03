use crate::data::{ReadResponse, StatusResponse};
use crate::operator::ApiOperator;
use actix_web::get;
use actix_web::web::{Data, Path};
use serde::Deserialize;
use tugboat_resources::manifests::core::v1::Ship;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub(super) struct ShipReadPathParams {
    namespace: String,
    name: String,
}

#[utoipa::path()]
#[get("/v1/namespaces/{namespace}/ships/{name}")]
pub(super) async fn handle_ship_read(
    path: Path<ShipReadPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<Ship>, StatusResponse> {
    let path = path.into_inner();
    let ship = operator
        .store
        .get(Some(path.namespace.clone()), &path.name)
        .await?;

    match ship {
        Some(data) => Ok(ReadResponse::new(data.apply_revision())),
        None => Err(StatusResponse::not_found(
            "Ship not found",
            Some(serde_json::json!({ "namespace": path.namespace, "name": path.name})),
        )),
    }
}
