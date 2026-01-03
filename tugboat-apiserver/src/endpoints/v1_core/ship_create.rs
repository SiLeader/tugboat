use crate::data::{CreateResponse, StatusResponse};
use crate::endpoints::NamespacedPathParams;
use crate::operator::ApiOperator;
use crate::{create_object, extract_object_meta};
use actix_web::post;
use actix_web::web::{Data, Json, Path};
use tugboat_resources::Resource;
use tugboat_resources::manifests::core::v1::Ship;

#[utoipa::path()]
#[post("/v1/namespaces/{namespace}/ships")]
pub(super) async fn handle_ship_create(
    path: Path<NamespacedPathParams>,
    json: Json<Ship>,
    operator: Data<ApiOperator>,
) -> Result<CreateResponse<Ship>, StatusResponse> {
    let ship = json.into_inner();

    let object_meta = extract_object_meta!(ship);
    let object_meta = operator.apply_namespace(object_meta, path.into_inner().namespace);

    create_object!(operator, object_meta, ship, Ship::type_meta())
}
