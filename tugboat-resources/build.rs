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

fn main() {
    let derives = if cfg!(feature = "schema") {
        "#[derive(::utoipa::ToSchema, ::serde::Serialize, ::serde::Deserialize)]"
    } else {
        "#[derive(::serde::Serialize, ::serde::Deserialize)]"
    };

    let mut config = prost_build::Config::default();

    // Apply default field attributes
    // Only applied to specific fields by name
    config.field_attribute("object_meta", "#[serde(rename = \"metadata\")]");
    config.field_attribute("type_meta", "#[serde(flatten)]");

    let resources = [
        ".tugboat.apps.v1",
        ".tugboat.authorization.v1",
        ".tugboat.core.v1",
        ".tugboat.coordination.v1",
        ".tugboat.meta.v1.ObjectMeta",
        ".tugboat.meta.v1.ObjectReference",
        ".tugboat.meta.v1.OwnerReference",
        ".tugboat.meta.v1.TypeMeta",
    ];

    for res in resources {
        config.type_attribute(res, derives);
        // Apply camelCase rename to struct/enum
        config.type_attribute(res, "#[serde(rename_all = \"camelCase\")]");
        // Apply default skip to fields within these types
        // Note: this applies the attribute to all fields in messages matching the path
        config.field_attribute(
            res,
            "#[serde(default, skip_serializing_if = \"crate::manifests::default\")]",
        );
    }

    // For Time, we only add ToSchema if needed
    if cfg!(feature = "schema") {
        config.type_attribute(".tugboat.meta.v1.Time", "#[derive(::utoipa::ToSchema)]");
    }

    config
        .compile_protos(
            &[
                // apps/v1
                "proto/apps/v1/workload.proto",
                "proto/apps/v1/replica_set.proto",
                "proto/apps/v1/deployment.proto",
                "proto/apps/v1/fleet.proto",
                // authorization/v1
                "proto/authorization/v1/policy_rule.proto",
                "proto/authorization/v1/role.proto",
                "proto/authorization/v1/cluster_role.proto",
                "proto/authorization/v1/role_binding.proto",
                "proto/authorization/v1/cluster_role_binding.proto",
                // core/v1
                "proto/core/v1/config_map.proto",
                "proto/core/v1/namespace.proto",
                "proto/core/v1/network_class.proto",
                "proto/core/v1/node.proto",
                "proto/core/v1/persistent_volume.proto",
                "proto/core/v1/persistent_volume_claim.proto",
                "proto/core/v1/secret.proto",
                "proto/core/v1/runtime_class.proto",
                "proto/core/v1/storage_class.proto",
                "proto/core/v1/ship.proto",
                "proto/core/v1/ship_class.proto",
                "proto/core/v1/service_account.proto",
                // meta/v1
                "proto/meta/v1/object_meta.proto",
                "proto/meta/v1/object_reference.proto",
                "proto/meta/v1/type_meta.proto",
                "proto/meta/v1/time.proto",
                // coordination/v1
                "proto/coordination/v1/lease.proto",
            ],
            &["proto"],
        )
        .unwrap();
}
