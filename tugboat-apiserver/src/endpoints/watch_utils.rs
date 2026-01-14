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

use crate::data::StatusResponse;
use crate::endpoints::WatchOption;
use crate::endpoints::selector::{FieldSelector, Selector};
use crate::operator::ApiOperator;
use actix_web::HttpResponse;
use actix_web_lab::respond::NdJson;
use async_stream::stream;
use serde::Serialize;
use tugboat_resource_store::serializer::StaticSerializable;
use tugboat_resources::ObjectMetaResource;

#[derive(Serialize)]
#[serde(tag = "type", content = "object")]
enum WatchEvent<T> {
    ADDED(T),
    MODIFIED(T),
    DELETED(T),
}

impl<T> WatchEvent<T> {
    fn content(&self) -> Option<&T> {
        match self {
            WatchEvent::ADDED(c) => Some(c),
            WatchEvent::MODIFIED(c) => Some(c),
            WatchEvent::DELETED(c) => Some(c),
        }
    }
}

pub(super) async fn watch<T>(
    operator: &ApiOperator,
    _options: WatchOption,
    field_selector: Option<Vec<Selector>>,
    label_selector: Option<Vec<Selector>>,
    resource_version: Option<String>,
    namespace: Option<String>,
) -> Result<HttpResponse, StatusResponse>
where
    T: 'static + ObjectMetaResource + StaticSerializable + Serialize,
{
    let field_selector = field_selector.map(|fs| {
        fs.into_iter()
            .map(Into::into)
            .collect::<Vec<FieldSelector>>()
    });

    let watch = operator
        .store
        .watch::<T>(resource_version, namespace)
        .await?;
    let stream = stream! {
        let mut watch = watch;
        loop {
            if watch.changed().await.is_err() {
                break;
            }
            let value = watch.borrow_and_update();

            for event in value.iter() {
                let event = WatchEvent::<T>::try_from(event.clone());
                match event {
                    Ok(event) => if check_selector(&event, &field_selector, &label_selector) {
                        yield Ok(event);
                    }
                    Err(err) => yield Err(err),
                }
            }
        }
    };
    Ok(HttpResponse::Ok().body(NdJson::new(stream).into_body_stream()))
}

fn check_selector<T>(
    event: &WatchEvent<T>,
    field_selector: &Option<Vec<FieldSelector>>,
    label_selector: &Option<Vec<Selector>>,
) -> bool
where
    T: ObjectMetaResource + Serialize,
{
    let Some(content) = event.content() else {
        return true;
    };
    if let Some(field_selector) = field_selector {
        let Ok(value) = serde_json::to_value(&content) else {
            return false;
        };
        if !field_selector.iter().all(|s| s.is_match(&value)) {
            return false;
        }
    }
    if let Some(label_selector) = label_selector {
        let Some(meta) = content.object_meta() else {
            return false;
        };
        if !label_selector.iter().all(|s| s.is_label_match(meta)) {
            return false;
        }
    }
    true
}

impl<T> TryFrom<tugboat_resource_store::watch::WatchEvent> for WatchEvent<T>
where
    T: StaticSerializable,
{
    type Error = StatusResponse;

    fn try_from(value: tugboat_resource_store::watch::WatchEvent) -> Result<Self, Self::Error> {
        match value {
            tugboat_resource_store::watch::WatchEvent::Added(value) => {
                let value = T::deserialize(value.value.as_slice())?;
                Ok(WatchEvent::ADDED(value))
            }
            tugboat_resource_store::watch::WatchEvent::Modified(value) => {
                let value = T::deserialize(value.value.as_slice())?;
                Ok(WatchEvent::MODIFIED(value))
            }
            tugboat_resource_store::watch::WatchEvent::Deleted(value) => {
                let value = T::deserialize(value.value.as_slice())?;
                Ok(WatchEvent::DELETED(value))
            }
        }
    }
}
