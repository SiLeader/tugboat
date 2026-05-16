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

pub mod v1 {
    use crate::validators::NameValidator;
    use crate::{apply_resource, apply_validators, resource_api};

    include!(concat!(env!("OUT_DIR"), "/tugboat.apps.v1.rs"));

    apply_resource!(Deployment, resource_api::DEPLOYMENT, namespaced);
    apply_resource!(ReplicaSet, resource_api::REPLICA_SET, namespaced);
    apply_resource!(Fleet, resource_api::FLEET, namespaced);

    apply_validators!(Deployment, validators NameValidator);
    apply_validators!(ReplicaSet, validators NameValidator);
    apply_validators!(Fleet, validators NameValidator);
}
