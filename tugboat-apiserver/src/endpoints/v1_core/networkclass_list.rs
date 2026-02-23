// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::data::{ResourceList, StatusResponse};
use crate::endpoints::selector::FilterBySelector;
use crate::endpoints::watch_utils::watch;
use crate::endpoints::{ListQuery, NamespacedPathParams};
use crate::operator::ApiOperator;
use actix_web::web::{Data, Path, Query};
use actix_web::{HttpResponse, get};
use tugboat_resources::manifests::core::v1::NetworkClass;

#[utoipa::path()]
#[get("/v1/namespaces/{namespace}/networkclasses")]
pub(super) async fn handle_networkclass_list(
    path: Path<NamespacedPathParams>,
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, StatusResponse> {
    let query = query.into_inner();
    let path = path.into_inner();
    handle_networkclass_list_impl(&operator, query, Some(path.namespace)).await
}

#[utoipa::path()]
#[get("/v1/networkclasses")]
pub(super) async fn handle_networkclass_list_all(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, StatusResponse> {
    let query = query.into_inner();
    handle_networkclass_list_impl(&operator, query, None).await
}

async fn handle_networkclass_list_impl(
    operator: &ApiOperator,
    query: ListQuery,
    namespace: Option<String>,
) -> Result<HttpResponse, StatusResponse> {
    if let Some(opts) = query.watch {
        watch::<NetworkClass>(
            &operator,
            opts,
            query.to_field_selector()?,
            query.to_label_selector()?,
            query.resource_version,
            namespace,
        )
        .await
    } else {
        let field_selector = query.to_field_selector()?;
        let label_selector = query.to_label_selector()?;
        let resources = operator
            .store
            .list::<NetworkClass>(namespace, None)
            .await?;
        Ok(ResourceList::from_serializable(
            resources
                .into_iter()
                .map(|d| d.apply_revision())
                .filter_by_selector(field_selector, label_selector),
        )?
        .into())
    }
}
