use crate::data::StatusResponse;
use crate::endpoints::WatchOption;
use crate::operator::ApiOperator;
use actix_web::HttpResponse;
use actix_web_lab::respond::NdJson;
use async_stream::stream;
use serde::Serialize;
use tugboat_resource_store::serializer::StaticSerializable;

#[derive(Serialize)]
#[serde(tag = "type", content = "object")]
enum WatchEvent<T> {
    ADDED(T),
    MODIFIED(T),
    DELETED(T),
}

pub(super) async fn watch<T>(
    operator: &ApiOperator,
    resource_version: Option<String>,
    _options: WatchOption,
) -> Result<HttpResponse, StatusResponse>
where
    T: 'static + StaticSerializable + Serialize,
{
    let watch = operator.store.watch::<T>(resource_version).await?;
    let stream = stream! {
        let mut watch = watch;
        loop {
            if watch.changed().await.is_err() {
                break;
            }
            let value = watch.borrow_and_update();

            for event in value.iter() {
                yield WatchEvent::<T>::try_from(event.clone());
            }
        }
    };
    Ok(HttpResponse::Ok().body(NdJson::new(stream).into_body_stream()))
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
