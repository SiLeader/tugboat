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

use actix_web::{HttpResponse, Responder, get};
use utoipa::OpenApi;
use utoipa_actix_web::service_config::ServiceConfig;

mod ship_snapshot;
mod volume_snapshot;
mod volume_snapshot_class;
mod volume_snapshot_content;

#[derive(OpenApi)]
#[openapi(
    paths(
        volume_snapshot::handle_volume_snapshot_create,
        volume_snapshot::handle_volume_snapshot_delete,
        volume_snapshot::handle_volume_snapshot_list,
        volume_snapshot::handle_volume_snapshot_list_all,
        volume_snapshot::handle_volume_snapshot_patch,
        volume_snapshot::handle_volume_snapshot_read,
        volume_snapshot::handle_volume_snapshot_replace,
        volume_snapshot::handle_volume_snapshot_status_patch,
        volume_snapshot::handle_volume_snapshot_status_replace,
        volume_snapshot_content::handle_volume_snapshot_content_create,
        volume_snapshot_content::handle_volume_snapshot_content_delete,
        volume_snapshot_content::handle_volume_snapshot_content_list,
        volume_snapshot_content::handle_volume_snapshot_content_patch,
        volume_snapshot_content::handle_volume_snapshot_content_read,
        volume_snapshot_content::handle_volume_snapshot_content_replace,
        volume_snapshot_content::handle_volume_snapshot_content_status_patch,
        volume_snapshot_content::handle_volume_snapshot_content_status_replace,
        volume_snapshot_class::handle_volume_snapshot_class_create,
        volume_snapshot_class::handle_volume_snapshot_class_delete,
        volume_snapshot_class::handle_volume_snapshot_class_list,
        volume_snapshot_class::handle_volume_snapshot_class_patch,
        volume_snapshot_class::handle_volume_snapshot_class_read,
        volume_snapshot_class::handle_volume_snapshot_class_replace,
        volume_snapshot_class::handle_volume_snapshot_class_status_patch,
        volume_snapshot_class::handle_volume_snapshot_class_status_replace,
        ship_snapshot::handle_ship_snapshot_create,
        ship_snapshot::handle_ship_snapshot_delete,
        ship_snapshot::handle_ship_snapshot_list,
        ship_snapshot::handle_ship_snapshot_list_all,
        ship_snapshot::handle_ship_snapshot_patch,
        ship_snapshot::handle_ship_snapshot_read,
        ship_snapshot::handle_ship_snapshot_replace,
        ship_snapshot::handle_ship_snapshot_status_patch,
        ship_snapshot::handle_ship_snapshot_status_replace,
    ),
    components(schemas(
        tugboat_resources::manifests::core::v1::ShipSnapshot,
        tugboat_resources::manifests::core::v1::ShipSnapshotSpec,
        tugboat_resources::manifests::core::v1::ShipSnapshotStatus,
        tugboat_resources::manifests::core::v1::ShipSnapshotVolumeRef,
        tugboat_resources::manifests::core::v1::ShipSnapshotCondition,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshot,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotSpec,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotSource,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotStatus,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotCondition,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotContent,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotContentSpec,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotContentSource,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotContentStatus,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotRef,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotClass,
        tugboat_resources::manifests::snapshot::v1::VolumeSnapshotClassSpec,
        tugboat_resources::manifests::core::v1::SecretReference,
        tugboat_resources::manifests::meta::v1::ObjectMeta,
        tugboat_resources::manifests::meta::v1::TypeMeta,
        tugboat_resources::manifests::meta::v1::Time,
    ))
)]
struct SnapshotV1ApiDoc;

#[get("/openapi/v3/apis/snapshot/v1")]
pub(crate) async fn openapi_snapshot_v1() -> impl Responder {
    HttpResponse::Ok().json(SnapshotV1ApiDoc::openapi())
}

pub(super) fn register_volume_snapshot(service: &mut ServiceConfig) {
    service
        .service(volume_snapshot::handle_volume_snapshot_create)
        .service(volume_snapshot::handle_volume_snapshot_delete)
        .service(volume_snapshot::handle_volume_snapshot_list)
        .service(volume_snapshot::handle_volume_snapshot_list_all)
        .service(volume_snapshot::handle_volume_snapshot_patch)
        .service(volume_snapshot::handle_volume_snapshot_read)
        .service(volume_snapshot::handle_volume_snapshot_replace)
        .service(volume_snapshot::handle_volume_snapshot_status_patch)
        .service(volume_snapshot::handle_volume_snapshot_status_replace);
}

pub(super) fn register_volume_snapshot_content(service: &mut ServiceConfig) {
    service
        .service(volume_snapshot_content::handle_volume_snapshot_content_create)
        .service(volume_snapshot_content::handle_volume_snapshot_content_delete)
        .service(volume_snapshot_content::handle_volume_snapshot_content_list)
        .service(volume_snapshot_content::handle_volume_snapshot_content_patch)
        .service(volume_snapshot_content::handle_volume_snapshot_content_read)
        .service(volume_snapshot_content::handle_volume_snapshot_content_replace)
        .service(volume_snapshot_content::handle_volume_snapshot_content_status_patch)
        .service(volume_snapshot_content::handle_volume_snapshot_content_status_replace);
}

pub(super) fn register_volume_snapshot_class(service: &mut ServiceConfig) {
    service
        .service(volume_snapshot_class::handle_volume_snapshot_class_create)
        .service(volume_snapshot_class::handle_volume_snapshot_class_delete)
        .service(volume_snapshot_class::handle_volume_snapshot_class_list)
        .service(volume_snapshot_class::handle_volume_snapshot_class_patch)
        .service(volume_snapshot_class::handle_volume_snapshot_class_read)
        .service(volume_snapshot_class::handle_volume_snapshot_class_replace)
        .service(volume_snapshot_class::handle_volume_snapshot_class_status_patch)
        .service(volume_snapshot_class::handle_volume_snapshot_class_status_replace);
}

pub(super) fn register_ship_snapshot(service: &mut ServiceConfig) {
    service
        .service(ship_snapshot::handle_ship_snapshot_create)
        .service(ship_snapshot::handle_ship_snapshot_delete)
        .service(ship_snapshot::handle_ship_snapshot_list)
        .service(ship_snapshot::handle_ship_snapshot_list_all)
        .service(ship_snapshot::handle_ship_snapshot_patch)
        .service(ship_snapshot::handle_ship_snapshot_read)
        .service(ship_snapshot::handle_ship_snapshot_replace)
        .service(ship_snapshot::handle_ship_snapshot_status_patch)
        .service(ship_snapshot::handle_ship_snapshot_status_replace);
}
