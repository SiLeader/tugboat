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

use crate::data::{ModifyResponse, ReadResponse, ResourceList, StatusResponse};
use crate::endpoints::ListQuery;
use crate::endpoints::selector::FilterBySelector;
use crate::endpoints::watch_utils::watch;
use crate::operator::ApiOperator;
use crate::{check_namespace_absent, create_object, extract_object_meta};
use actix_web::HttpResponse;
use actix_web::web::{Data, Json, Path, Query};
use serde::{Deserialize, Serialize};
use tugboat_resource_store::serializer::StaticSerializable;
use tugboat_resources::validators::Validatable;
use tugboat_resources::{ObjectMetaResource, SetTypeMeta, StaticResource};
use utoipa::ToSchema;

pub(super) fn register_cluster_scoped_resource<T>(
    config: &mut utoipa_actix_web::service_config::ServiceConfig,
) where
    T: 'static,
    T: StaticResource + ObjectMetaResource + StaticSerializable + SetTypeMeta,
    T: Serialize,
    T: Clone,
{
    let list_path = format!("/{}/{}", T::version(), T::plural());
}

pub(super) async fn handle_cluster_scoped_resource_create<T>(
    json: Json<T>,
    operator: Data<ApiOperator>,
) -> Result<ModifyResponse<T>, StatusResponse>
where
    T: StaticResource + ObjectMetaResource + StaticSerializable + SetTypeMeta + Clone,
    T: Validatable,
{
    let resource = json.into_inner();

    if resource.validate() {
        return Err(StatusResponse::bad_request("", None));
    }

    let object_meta = extract_object_meta!(resource);
    check_namespace_absent!(object_meta);
    let object_meta = operator.apply_uid(object_meta);

    create_object!(operator, object_meta, resource, T::type_meta())
}

pub(super) async fn handle_cluster_scoped_resource_list<T>(
    query: Query<ListQuery>,
    operator: Data<ApiOperator>,
) -> Result<HttpResponse, StatusResponse>
where
    T: 'static + StaticResource + StaticSerializable + ObjectMetaResource + Serialize,
{
    let query = query.into_inner();
    if let Some(opts) = query.watch {
        watch::<T>(
            &operator,
            opts,
            query.to_field_selector()?,
            query.to_label_selector()?,
            query.resource_version,
            None,
        )
        .await
    } else {
        let field_selector = query.to_field_selector()?;
        let label_selector = query.to_label_selector()?;
        let resources = operator.store.list::<T>(None, None).await?;
        Ok(ResourceList::from_serializable(
            resources
                .into_iter()
                .map(|d| d.apply_revision())
                .filter_by_selector(field_selector, label_selector),
        )?
        .into())
    }
}

#[derive(Deserialize, ToSchema)]
struct ReadParams {
    name: String,
}

pub(super) async fn handle_cluster_scoped_resource_read<T>(
    path: Path<ReadParams>,
    operator: Data<ApiOperator>,
) -> Result<ReadResponse<T>, StatusResponse>
where
    T: StaticSerializable + ObjectMetaResource + Serialize,
{
    let resource = operator.store.get(None, &path.name).await?;

    match resource {
        Some(data) => Ok(ReadResponse::new(data.apply_revision())),
        None => Err(StatusResponse::not_found(
            "Namespace not found",
            Some(serde_json::json!({ "name": path.name })),
        )),
    }
}
