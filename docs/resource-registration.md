# API Resource Registration

Tugboat resource metadata is centralized in `tugboat-resources/src/resource_api.rs`.
Treat that file as the source of truth for API group, version, kind, plural name,
singular name, scope, enabled verbs, and status subresource support.

The descriptor table is consumed by generated resource traits, the apiserver route
registry, discovery output, RBAC resource/verb strings, and serializer coverage
tests. Do not add a second metadata table for a new resource.

## Current Groups

| Group/version | Scope | Resources |
| --- | --- | --- |
| `core/v1` | mixed | `ConfigMap`, `Namespace`, `NetworkClass`, `ClusterNetworkClass`, `Node`, `PersistentVolume`, `PersistentVolumeClaim`, `Secret`, `ServiceAccount`, `RuntimeClass`, `StorageClass`, `Ship`, `ShipClass` |
| `apps/v1` | namespaced | `Deployment`, `ReplicaSet`, `Fleet` |
| `authorization/v1` | mixed | `ClusterRole`, `ClusterRoleBinding`, `Role`, `RoleBinding` |
| `coordination/v1` | namespaced | `Lease` |
| `snapshot/v1` | mixed | `VolumeSnapshot`, `VolumeSnapshotContent`, `VolumeSnapshotClass`, `ShipSnapshot` |

## Adding a Resource

1. Define the protobuf message under `tugboat-resources/proto/{group}/v1/` and include it from `tugboat-resources/build.rs`.
2. Add one `ResourceApiDescriptor` constant in `tugboat-resources/src/resource_api.rs`, then include it in `ALL_RESOURCE_DESCRIPTORS` in the intended discovery order.
3. Register the generated type in `tugboat-resources/src/manifests/mod.rs` with `apply_resource!(TypeName, resource_api::DESCRIPTOR_NAME, namespaced)` or `cluster`.
4. Add validators in `manifests/mod.rs` with `apply_validators!`. Cluster-scoped resources should reject namespace metadata.
5. Add endpoint wrappers under `tugboat-apiserver/src/endpoints/v1_{group}/`. Keep wrappers thin and route behavior aligned with the generic resource handlers.
6. Wire the descriptor to the endpoint wrapper in `tugboat-apiserver/src/endpoints/resource_registry.rs`. This is what makes discovery and route coverage line up with the descriptor table.
7. Add protobuf serialization in `tugboat-resource-store/src/serializer/mod.rs` with `protobuf_serializable!`.
8. If the resource is controller-owned, add its controller separately and keep status updates behind the status subresource when the descriptor exposes one.
9. Update public docs and sample manifests when the resource is user-facing.

## Namespaced and Cluster-scoped Pairs

Some resources follow a pair pattern where a namespaced user-facing resource binds to a cluster-scoped provider-facing resource. A classic example is `VolumeSnapshot` (namespaced) and `VolumeSnapshotContent` (cluster-scoped), similar to `PersistentVolumeClaim` and `PersistentVolume`.

When adding such pairs:
- Use `spec.source` in the namespaced resource to reference the cluster-scoped resource (static binding).
- Use `spec.claimRef` in the cluster-scoped resource to reference the namespaced resource (back-binding).
- Ensure validators check for cross-namespace references and enforce that cluster-scoped resources reject namespace metadata.

## Cross-group Type Reuse

If shared types (like `LabelSelector` or `Condition`) are introduced in a common file like `tugboat-resources/proto/core/v1/selector.proto`, they can be reused across different API groups.

- Import the shared proto file in your group-specific proto: `import "core/v1/selector.proto";`
- Use the fully qualified type name if necessary.
- Ensure the shared proto is included in `tugboat-resources/build.rs` before the groups that depend on it.
- This allows for a consistent API contract across different resource groups (e.g., sharing selectors between `apps/v1` and `snapshot/v1`).

## Status Subresources

Set `status_patch` and `status_update` in `ResourceOperations` only when the resource has a status contract. The descriptor should then expose status verbs, the apiserver wrapper should register status patch/replace routes, and RBAC should grant `resources/status` separately from the main resource.

## Contract Tests

These tests should fail if resource metadata drifts between layers:

- `tugboat-resources`: `StaticResource` metadata matches the descriptor table.
- `tugboat-resource-store`: every descriptor has serializer coverage.
- `tugboat-apiserver`: route registry, discovery entries, status routes, and RBAC resource strings are derived from the same descriptors.

Run the API/resource gate after changing resource registration:

```bash
cargo test -p tugboat-resources
cargo test -p tugboat-resource-store
cargo test -p tugboat-apiserver
cargo test -p tugboat-integration-tests api_discovery
cargo test -p tugboat-integration-tests resource_versioning
cargo test -p tugboat-integration-tests watch
```
