use crate::data::{ResourceList, StatusResponse};
use crate::operator::ApiOperator;
use actix_web::get;
use actix_web::web::Data;
use tugboat_resources::manifests::core::v1::ShipClass;

#[utoipa::path()]
#[get("/v1/shipclasses")]
pub(super) async fn handle_shipclass_list(
    operator: Data<ApiOperator>,
) -> Result<ResourceList, StatusResponse> {
    let shipclasses = operator.store.list::<ShipClass>(None, None).await?;
    ResourceList::from_serializable(
        shipclasses
            .into_iter()
            .map(|d| d.apply_revision())
            .collect(),
    )
}
