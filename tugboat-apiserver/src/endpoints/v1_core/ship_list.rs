use crate::data::{ResourceList, StatusResponse};
use crate::endpoints::NamespacedPathParams;
use crate::operator::ApiOperator;
use actix_web::get;
use actix_web::web::{Data, Path};
use tugboat_resources::manifests::core::v1::Ship;

#[utoipa::path()]
#[get("/v1/namespaces/{namespace}/ships")]
pub(super) async fn handle_ship_list(
    path: Path<NamespacedPathParams>,
    operator: Data<ApiOperator>,
) -> Result<ResourceList, StatusResponse> {
    let ships = operator
        .store
        .list::<Ship>(Some(path.into_inner().namespace), None)
        .await?;
    ResourceList::from_serializable(ships.into_iter().map(|s| s.apply_revision()).collect())
}
