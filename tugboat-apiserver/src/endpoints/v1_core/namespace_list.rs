use crate::data::{ResourceList, StatusResponse};
use crate::endpoints::ListQuery;
use crate::endpoints::watch_utils::watch;
use crate::operator::ApiOperator;
use actix_web::web::{Data, Query};
use actix_web::{HttpResponse, get};
use tugboat_resources::manifests::core::v1::Namespace;

#[utoipa::path()]
#[get("/v1/namespaces")]
pub(super) async fn handle_namespace_list(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, StatusResponse> {
    let query = query.into_inner();
    if let Some(opts) = query.watch {
        watch::<Namespace>(&operator, query.resource_version, opts).await
    } else {
        let namespaces = operator.store.list::<Namespace>(None, None).await?;
        Ok(ResourceList::from_serializable(
            namespaces.into_iter().map(|d| d.apply_revision()).collect(),
        )?
        .into())
    }
}
