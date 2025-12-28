use crate::data::{ResourceList, StatusResponse};
use crate::operator::ApiOperator;
use actix_web::get;
use actix_web::web::Data;
use tugboat_resources::manifests::core::v1::Namespace;

#[utoipa::path()]
#[get("/v1/namespaces")]
pub(super) async fn handle_namespace_list(
    operator: Data<ApiOperator>,
) -> Result<ResourceList, StatusResponse> {
    let namespaces = operator.store.list::<Namespace>(None, None).await?;
    ResourceList::from_serializable(namespaces.into_iter().map(|d| d.apply_revision()).collect())
}
