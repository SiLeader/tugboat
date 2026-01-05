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
use crate::endpoints::ListQuery;
use crate::endpoints::selector::FilterBySelector;
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
        watch::<ShipClass>(
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
        let shipclasses = operator.store.list::<ShipClass>(None, None).await?;
        Ok(ResourceList::from_serializable(
            shipclasses
                .into_iter()
                .map(|d| d.apply_revision())
                .filter_by_selector(field_selector, label_selector),
        )?
        .into())
    }
}
