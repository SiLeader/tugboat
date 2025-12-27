fn main() {
    prost_build::Config::default()
        .type_attribute(".", "#[derive(::serde::Serialize, ::serde::Deserialize)]")
        .message_attribute(".", "#[serde(rename_all = \"camelCase\")]")
        .enum_attribute(".", "#[serde(rename_all = \"PascalCase\"")
        .field_attribute("object_meta", "#[serde(rename = \"metadata\")]")
        .field_attribute("type_meta", "#[serde(flatten)]")
        .compile_protos(
            &[
                // core/v1
                "proto/core/v1/namespace.proto",
                "proto/core/v1/node.proto",
                "proto/core/v1/ship.proto",
                "proto/core/v1/ship_class.proto",
                // meta/v1
                "proto/meta/v1/object_meta.proto",
                "proto/meta/v1/time.proto",
                "proto/meta/v1/type_meta.proto",
            ],
            &["proto"],
        )
        .unwrap();
}
