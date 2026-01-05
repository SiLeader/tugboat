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

pub mod core {
    pub mod v1 {
        use crate::apply_resource;

        include!(concat!(env!("OUT_DIR"), "/tugboat.core.v1.rs"));

        apply_resource!(Namespace, "core", "v1", "namespaces", "namespace", cluster);
        apply_resource!(Node, "core", "v1", "nodes", "node", cluster);
        apply_resource!(Ship, "core", "v1", "ships", "ship", namespaced);
        apply_resource!(ShipClass, "core", "v1", "shipclasses", "shipclass", cluster);
    }
}

pub mod meta {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/tugboat.meta.v1.rs"));
    }
}
