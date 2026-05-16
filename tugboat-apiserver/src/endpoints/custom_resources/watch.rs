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

use super::dispatch::{custom_data_to_value, matches_selectors};
use crate::crd_registry::CrdEntry;
use crate::data::StatusResponse;
use crate::endpoints::ListQuery;
use crate::endpoints::selector::Selector;
use crate::operator::ApiOperator;
use actix_web::HttpResponse;
use actix_web_lab::respond::NdJson;
use async_stream::stream;
use serde::Serialize;
use tugboat_resource_store::ContentData;
use tugboat_resource_store::serializer::custom_resource::CustomResourceSerializable;
use tugboat_resources::manifests::meta::v1::CustomResourceObject;

#[derive(Serialize)]
#[serde(tag = "type", content = "object")]
enum CustomWatchEvent {
    #[serde(rename = "ADDED")]
    Added(serde_json::Value),
    #[serde(rename = "MODIFIED")]
    Modified(serde_json::Value),
    #[serde(rename = "DELETED")]
    Deleted(serde_json::Value),
}

pub(super) async fn watch_custom(
    operator: &ApiOperator,
    entry: CrdEntry,
    namespace: Option<String>,
    query: ListQuery,
    _watch: crate::endpoints::WatchOption,
) -> Result<HttpResponse, Box<StatusResponse>> {
    let field_selector = query.to_field_selector()?;
    let label_selector = query.to_label_selector()?;
    let resource_version = query
        .resource_version
        .as_deref()
        .map(str::parse::<i64>)
        .transpose()
        .map_err(|_| {
            Box::new(StatusResponse::bad_request(
                "resourceVersion must be a valid integer",
                None,
            ))
        })?;

    let watch = operator
        .store
        .watch_custom(
            &entry.group,
            &entry.plural,
            namespace.as_deref(),
            resource_version,
        )
        .await?;
    let stream = stream! {
        let mut watch = watch;
        loop {
            if watch.changed().await.is_err() {
                break;
            }
            let events = watch.borrow_and_update();
            for event in events.iter() {
                match custom_watch_event(event.clone(), &field_selector, &label_selector) {
                    Ok(Some(event)) => yield Ok(event),
                    Ok(None) => {}
                    Err(err) => yield Err(err),
                }
            }
        }
    };
    Ok(HttpResponse::Ok().body(NdJson::new(stream).into_body_stream()))
}

fn custom_watch_event(
    event: tugboat_resource_store::watch::WatchEvent,
    field_selector: &Option<Vec<Selector>>,
    label_selector: &Option<Vec<Selector>>,
) -> Result<Option<CustomWatchEvent>, Box<StatusResponse>> {
    match event {
        tugboat_resource_store::watch::WatchEvent::Added(kv) => {
            let value = custom_kv_to_value(kv)?;
            if matches_selectors(&value, field_selector, label_selector) {
                Ok(Some(CustomWatchEvent::Added(value)))
            } else {
                Ok(None)
            }
        }
        tugboat_resource_store::watch::WatchEvent::Modified(kv) => {
            let value = custom_kv_to_value(kv)?;
            if matches_selectors(&value, field_selector, label_selector) {
                Ok(Some(CustomWatchEvent::Modified(value)))
            } else {
                Ok(None)
            }
        }
        tugboat_resource_store::watch::WatchEvent::Deleted(kv) => {
            let value = custom_kv_to_value(kv)?;
            if matches_selectors(&value, field_selector, label_selector) {
                Ok(Some(CustomWatchEvent::Deleted(value)))
            } else {
                Ok(None)
            }
        }
    }
}

fn custom_kv_to_value(
    kv: tugboat_resource_store::watch::KeyValue,
) -> Result<serde_json::Value, Box<StatusResponse>> {
    let envelope = CustomResourceObject::deserialize(kv.value.as_slice())?;
    custom_data_to_value(ContentData {
        data: envelope,
        revision: kv.revision,
    })?
    .ok_or_else(|| {
        Box::new(StatusResponse::internal_error(
            "Missing custom resource body",
            None,
        ))
    })
}
