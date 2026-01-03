use crate::data::{ResourceList, StatusResponse};
use crate::endpoints::ListQuery;
use crate::endpoints::watch_utils::watch;
use crate::operator::ApiOperator;
use actix_web::web::{Data, Query};
use actix_web::{HttpResponse, get};
use tugboat_resources::manifests::core::v1::ShipClass;

#[utoipa::path()]
#[get("/v1/shipclasses")]
pub(super) async fn handle_shipclass_list(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, StatusResponse> {
    let query = query.into_inner();
    if let Some(opts) = query.watch {
        watch::<ShipClass>(&operator, query.resource_version, opts).await
    } else {
        let shipclasses = operator.store.list::<ShipClass>(None, None).await?;
        Ok(ResourceList::from_serializable(
            shipclasses
                .into_iter()
                .map(|d| d.apply_revision())
                .collect(),
        )?
        .into())
    }
}
