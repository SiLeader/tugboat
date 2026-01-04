use crate::data::{ResourceList, StatusResponse};
use crate::endpoints::selector::FilterBySelector;
use crate::endpoints::watch_utils::watch;
use crate::endpoints::{ListQuery, NamespacedPathParams};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Path, Query};
use actix_web::{HttpResponse, get};
use tugboat_resources::manifests::core::v1::Ship;

#[utoipa::path()]
#[get("/v1/namespaces/{namespace}/ships")]
pub(super) async fn handle_ship_list(
    path: Path<NamespacedPathParams>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, StatusResponse> {
    let query = query.into_inner();
    if let Some(opts) = query.watch {
        watch::<Ship>(&operator, query.resource_version, opts).await
    } else {
        let field_selector = query.to_field_selector()?;
        let label_selector = query.to_label_selector()?;
        let ships = operator
            .store
            .list::<Ship>(Some(path.into_inner().namespace), None)
            .await?;
        Ok(ResourceList::from_serializable(
            ships
                .into_iter()
                .map(|s| s.apply_revision())
                .filter_by_selector(field_selector, label_selector),
        )?
        .into())
    }
}
