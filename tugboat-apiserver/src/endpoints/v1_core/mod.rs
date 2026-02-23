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

use utoipa_actix_web::service_config::ServiceConfig;

mod clusternetworkclass_create;
mod clusternetworkclass_list;
mod clusternetworkclass_read;
mod namespace_create;
mod namespace_list;
mod namespace_read;
mod networkclass_create;
mod networkclass_list;
mod networkclass_read;
mod ship_create;
mod ship_list;
mod ship_read;
mod ship_status_patch;
mod ship_status_replace;
mod shipclass_create;
mod shipclass_list;
mod shipclass_read;

pub(super) fn register_v1_core(service: &mut ServiceConfig) {
    service
        .service(clusternetworkclass_create::handle_clusternetworkclass_create)
        .service(clusternetworkclass_list::handle_clusternetworkclass_list)
        .service(clusternetworkclass_read::handle_clusternetworkclass_read)
        .service(namespace_create::handle_namespace_create)
        .service(namespace_list::handle_namespace_list)
        .service(namespace_read::handle_namespace_read)
        .service(networkclass_create::handle_networkclass_create)
        .service(networkclass_list::handle_networkclass_list)
        .service(networkclass_list::handle_networkclass_list_all)
        .service(networkclass_read::handle_networkclass_read)
        .service(ship_create::handle_ship_create)
        .service(ship_list::handle_ship_list)
        .service(ship_list::handle_ship_list_all)
        .service(ship_read::handle_ship_read)
        .service(ship_status_patch::handle_ship_status_patch)
        .service(ship_status_replace::handle_ship_status_replace)
        .service(shipclass_create::handle_shipclass_create)
        .service(shipclass_list::handle_shipclass_list)
        .service(shipclass_read::handle_shipclass_read);
}
