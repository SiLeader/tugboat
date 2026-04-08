# Copilot Instructions for Tugboat

## Build, test, and lint commands

```bash
# Workspace
cargo build --release
cargo test

# Single package
cargo build --release --package tugboat-apiserver
cargo test --package tugboat-resources

# Single test examples (name filter)
cargo test --package tugboat-resources can_compare_resource_version_1
cargo test --package tugboat-apiserver can_parse_equality

# Lint / format / audit
cargo clippy
cargo fmt --check
cargo fmt
cargo deny check
```

### Final checks (pre-merge)

As a final verification before merging or releasing, run the following commands and address any issues they report:

- cargo clippy
- cargo fmt --check
- cargo test


## High-level architecture

- Tugboat is a Rust workspace for Kubernetes-style VM orchestration with etcd as the source of truth.
- `tugboat-resources` defines API types as protobuf (`proto/**`) and generates Rust types/traits consumed across crates.
- `tugboat-apiserver` (Actix Web) exposes REST endpoints and persists/reads resources through `tugboat-resource-store`.
- `tugboat-resource-store` wraps etcd CRUD + watch and stores objects under `/tugboat/registry/{group}/{plural}/...`.
- `tugboat-agent` watches `Ship` resources (field selector `spec.nodeName=<node>`), reconciles desired state, updates ship status, and orchestrates runtime + networking.
- `tugboat-scheduler` watches for unscheduled `Ship` resources and assigns them to nodes based on resource availability and constraints.
- `tugboat-controller-manager` runs multiple reconciliation controllers: NetworkClassStatus, PvcProvisioner, Fleet, Deployment, ReplicaSet, and PersistentVolumeCleanup.
- `tugboat-csi-operator` provides CSI gRPC client logic used by the agent and controller-manager for storage operations.
- `tugboat-vm-image` handles OCI VM image pull/push, and `tugboat-vm-runtime-interface` shells out to `tugboat-runtime`.
- `tugboat-runtime` is the QEMU executor CLI used by the agent/runtime interface (`create`, `start`, `status` flow).
- Kubernetes concept mapping used in code/docs: Pod → Ship, Deployment → Deployment, ReplicaSet → ReplicaSet, DaemonSet → Fleet, container image → VM image (OCI), Dockerfile → Imagefile.

## Key conventions

- Add/modify API resources by editing protobufs in `tugboat-resources/proto/{group}/v1/` and updating `tugboat-resources/build.rs` compile list.
- Register resource metadata/scope traits with `apply_resource!` and validators with `apply_validators!` in `tugboat-resources/src/manifests/mod.rs`.
- Protobuf-generated manifest serialization is opinionated: camelCase fields, `object_meta` serialized as `metadata`, and `type_meta` flattened.
- Endpoint files in `tugboat-apiserver/src/endpoints/v1_{group}/` are split by action (`*_create.rs`, `*_list.rs`, `*_read.rs`, status patch/replace) and must be registered in `endpoints/resource_registry.rs`.
- All resources must also be added to `tugboat-resource-store/src/serializer/mod.rs` via `protobuf_serializable!`.
- Route shape follows resource scope:
  - Cluster-scoped: `/v1/{plural}`, `/v1/{plural}/{name}`
  - Namespaced: `/v1/namespaces/{namespace}/{plural}`, `/v1/namespaces/{namespace}/{plural}/{name}`, plus `/v1/{plural}` for list-all where implemented.
- Reuse endpoint helper macros in `tugboat-apiserver/src/endpoints/utils.rs` for create handlers: `extract_object_meta!`, `check_namespace_absent!`, `create_object!`.
- Component configs are TOML; default CLI config paths are `/etc/tugboat/{apiserver|agent|scheduler|controller-manager|runtime}/config.toml`, with examples in `sample-configs/`.

### Editing guidance

- Recommended: Keep file length to about 800 lines maximum. Very large files are harder to review and understand.
- Recommended (not mandatory): Keep individual functions to approximately 100 lines or less when possible. Prefer splitting complex logic into smaller functions to improve readability and testability.

Note: The function-size guideline is a recommendation, not a strict rule — apply it flexibly based on context.

