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

mod lease_create;
mod lease_list;
mod lease_read;
mod lease_replace;

pub(super) fn register_lease(service: &mut ServiceConfig) {
    service
        .service(lease_create::handle_lease_create)
        .service(lease_list::handle_lease_list)
        .service(lease_list::handle_lease_list_all)
        .service(lease_read::handle_lease_read)
        .service(lease_replace::handle_lease_replace);
}

#[allow(dead_code)]
pub(super) fn register_v1_coordination(service: &mut ServiceConfig) {
    service.configure(register_lease);
}
