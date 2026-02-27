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

## High-level architecture

- Tugboat is a Rust workspace for Kubernetes-style VM orchestration with etcd as the source of truth.
- `tugboat-resources` defines API types as protobuf (`proto/**`) and generates Rust types/traits consumed across crates.
- `tugboat-apiserver` (Actix Web) exposes REST endpoints and persists/read resources through `tugboat-resource-store`.
- `tugboat-resource-store` wraps etcd CRUD + watch and stores objects under `/tugboat/registry/{group}/{plural}/...`.
- `tugboat-agent` watches `Ship` resources (field selector `spec.nodeName=<node>`), reconciles desired state, updates ship status, and orchestrates runtime + networking.
- `tugboat-vm-image` handles OCI VM image pull/push, and `tugboat-vm-runtime-interface` shells out to `tugboat-runtime`.
- `tugboat-runtime` is the QEMU executor CLI used by the agent/runtime interface (`create`, `start`, `status` flow).
- Kubernetes concept mapping used in code/docs: Pod -> Ship, Deployment -> Fleet (planned), container image -> VM image (OCI), Dockerfile -> Imagefile.

## Key conventions

- Add/modify API resources by editing protobufs in `tugboat-resources/proto/**` and updating `tugboat-resources/build.rs` compile list.
- Register resource metadata/scope traits with `apply_resource!` and validators with `apply_validators!` in `tugboat-resources/src/manifests/mod.rs`.
- Protobuf-generated manifest serialization is opinionated: camelCase fields, `object_meta` serialized as `metadata`, and `type_meta` flattened.
- Endpoint files in `tugboat-apiserver/src/endpoints/v1_core/` are split by action (`*_create.rs`, `*_list.rs`, `*_read.rs`, status patch/replace) and must be wired in `v1_core/mod.rs`.
- Route shape follows resource scope:
  - Cluster-scoped: `/v1/{plural}`, `/v1/{plural}/{name}`
  - Namespaced: `/v1/namespaces/{namespace}/{plural}`, `/v1/namespaces/{namespace}/{plural}/{name}`, plus `/v1/{plural}` for list-all where implemented.
- Reuse endpoint helper macros in `tugboat-apiserver/src/endpoints/utils.rs` for create handlers: `extract_object_meta!`, `check_namespace_absent!`, `create_object!`.
- Component configs are TOML; default CLI config paths are `/etc/tugboat/{apiserver|agent|runtime}/config.toml`, with examples in `sample-configs/`.
