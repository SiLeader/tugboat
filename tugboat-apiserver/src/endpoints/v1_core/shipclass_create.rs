use crate::data::{CreateResponse, StatusResponse};
use crate::operator::ApiOperator;
use crate::{check_namespace_absent, create_object, extract_object_meta};
use actix_web::post;
use actix_web::web::{Data, Json};
use tugboat_resources::Resource;
use tugboat_resources::manifests::core::v1::ShipClass;

#[utoipa::path()]
#[post("/v1/shipclasses")]
pub(super) async fn handle_shipclass_create(
    json: Json<ShipClass>,
    operator: Data<ApiOperator>,
) -> Result<CreateResponse<ShipClass>, StatusResponse> {
    let shipclass = json.into_inner();

    let object_meta = extract_object_meta!(shipclass);
    check_namespace_absent!(object_meta);
    let object_meta = operator.apply_uid(object_meta);

    create_object!(operator, object_meta, shipclass, ShipClass::type_meta())
}
