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
    prost_build::Config::default()
        .type_attribute(
            ".",
            if cfg!(feature = "schema") {
                "#[derive(::utoipa::ToSchema, ::serde::Serialize, ::serde::Deserialize)]"
            } else {
                "#[derive(::serde::Serialize, ::serde::Deserialize)]"
            },
        )
        .message_attribute(".", "#[serde(rename_all = \"camelCase\")]")
        .enum_attribute(".", "#[serde(rename_all = \"PascalCase\"")
        .field_attribute("object_meta", "#[serde(rename = \"metadata\")]")
        .field_attribute("type_meta", "#[serde(flatten)]")
        .field_attribute(
            ".",
            "#[serde(default, skip_serializing_if = \"crate::manifests::default\")]",
        )
        .compile_protos(
            &[
                // core/v1
                "proto/core/v1/namespace.proto",
                "proto/core/v1/network_class.proto",
                "proto/core/v1/node.proto",
                "proto/core/v1/ship.proto",
                "proto/core/v1/ship_class.proto",
                // meta/v1
                "proto/meta/v1/object_meta.proto",
                "proto/meta/v1/time.proto",
                "proto/meta/v1/type_meta.proto",
                // coordination/v1
                "proto/coordination/v1/lease.proto",
            ],
            &["proto"],
        )
        .unwrap();
}
