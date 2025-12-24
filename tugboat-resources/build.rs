fn main() {
    prost_build::compile_protos(
        &[
            // core/v1
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
