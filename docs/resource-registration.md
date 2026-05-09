# API Resource Registration

Tugboat resource metadata is centralized in
`tugboat-resources/src/resource_api.rs`. Treat that file as the source of truth
for API group, version, kind, plural name, scope, enabled verbs, and status
subresource support.

## Adding a Resource

1. Define the protobuf message under `tugboat-resources/proto/{group}/v1/` and
   include it from `tugboat-resources/build.rs`.
2. Add one `ResourceApiDescriptor` constant in
   `tugboat-resources/src/resource_api.rs`, then include it in
   `ALL_RESOURCE_DESCRIPTORS`.
3. Register the generated type in `tugboat-resources/src/manifests/mod.rs` with
   `apply_resource!(TypeName, resource_api::DESCRIPTOR_NAME, namespaced)` or
   `cluster`.
4. Add validators in `manifests/mod.rs` with `apply_validators!`.
5. Add endpoint wrappers under `tugboat-apiserver/src/endpoints/v1_{group}/`
   and wire the descriptor to the wrapper in
   `tugboat-apiserver/src/endpoints/resource_registry.rs`.
6. Add protobuf serialization in
   `tugboat-resource-store/src/serializer/mod.rs` with
   `protobuf_serializable!`.

## Contract Tests

These tests should fail if resource metadata drifts between layers:

- `tugboat-resources`: `StaticResource` metadata matches the descriptor table.
- `tugboat-resource-store`: every descriptor has serializer coverage.
- `tugboat-apiserver`: route registry, discovery entries, and RBAC resource
  strings are derived from the same descriptors.

Run the API/resource gate after changing resource registration:

```bash
cargo test -p tugboat-resources
cargo test -p tugboat-resource-store
cargo test -p tugboat-apiserver
```
