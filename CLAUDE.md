# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
# Build entire workspace
cargo build --release

# Build a single package
cargo build --release --package tugboat-apiserver

# Run tests
cargo test
cargo test --package tugboat-resources

# Lint & format
cargo clippy
cargo fmt --check
cargo fmt  # auto-format

# Dependency security audit (uses deny.toml)
cargo deny check
```

### Final checks (pre-merge)

As a final verification before merging or releasing, run the following commands and address any issues they report:

- cargo clippy
- cargo fmt --check
- cargo test


Packages: `tugboat-resources`, `tugboat-apiserver`, `tugboat-agent`, `tugboat-runtime`,
`tugboat-resource-store`, `tugboat-client`, `tugboat-cli`, `tugboat-vm-image`,
`tugboat-vm-runtime-interface`, `tugboat-cni-operator`, `tugboat-csi-operator`, `tugboat-scheduler`

## Architecture

Tugboat is a Kubernetes-inspired VM orchestration system written in Rust. It manages VMs declaratively
using etcd as the single source of truth, with direct QEMU execution (no libvirt).

### Component Dependency Graph

```
tugboat-apiserver (actix-web REST API)
  ├─ tugboat-resources (with "schema" feature for OpenAPI)
  └─ tugboat-resource-store (etcd wrapper)

tugboat-agent (node reconciler)
  ├─ tugboat-client (HTTP client)
  ├─ tugboat-resources
  ├─ tugboat-vm-image (OCI image handling)
  ├─ tugboat-vm-runtime-interface (runtime bridge)
  └─ tugboat-cni-operator (CNI networking)

tugboat-scheduler (pod scheduler)
  ├─ tugboat-client
  └─ tugboat-resources

tugboat-csi-operator (storage operator)
  └─ tugboat-resources (implicit via proto)

tugboat-runtime (QEMU executor)
  ├─ tugboat-resources
  └─ tugboat-vm-runtime-interface
```

### Resource System (tugboat-resources)

All API types are defined as protobuf in `tugboat-resources/proto/` and compiled via `build.rs`
using prost-build. The build script applies serde + optional utoipa (OpenAPI) derives.

Resources implement traits via the `apply_resource!` macro in `tugboat-resources/src/manifests/mod.rs`:
- `StaticResource` – group, version, kind, plural, singular
- `ClusterScopedResource` or `NamespacedResource` – scope marker
- `ObjectMetaResource` – metadata accessor
- `SetTypeMeta` – type meta setter

Validators are applied via `apply_validators!` macro (e.g., `NameValidator`, `NamespaceProhibitedValidator`).

**Kubernetes concept mapping:** Pod→Ship, Deployment→Fleet, Container image→VM image (OCI), Dockerfile→Imagefile

### API Server (tugboat-apiserver)

Endpoints live in `tugboat-apiserver/src/endpoints/v1_core/`. Each resource has separate files for
create, list, and read operations. Registration happens in `endpoints/v1_core/mod.rs` →
`endpoints/mod.rs` → mounted under the configured path in `lib.rs`.

Key patterns:
- **Cluster-scoped resources** (ShipClass, Namespace, Node, PersistentVolume, ClusterNetworkClass): route pattern `/v1/{plural}` and `/v1/{plural}/{name}`
- **Namespaced resources** (Ship, PersistentVolumeClaim, Secret, NetworkClass, Lease): route pattern `/v1/namespaces/{namespace}/{plural}` and `/v1/namespaces/{namespace}/{plural}/{name}`, plus `/v1/{plural}` for list-all
- Macros in `endpoints/utils.rs`: `extract_object_meta!`, `check_namespace_absent!`, `create_object!`
- Generic handlers in `endpoints/v1_core/cluster_resources.rs` for cluster-scoped CRUD
- `ApiOperator` (in `operator.rs`) wraps `ResourceStore` + `NameGenerator`

### Adding a New API Resource

1. Define protobuf in `tugboat-resources/proto/core/v1/` and add to `build.rs` compile list
2. Register with `apply_resource!` and `apply_validators!` in `tugboat-resources/src/manifests/mod.rs`
3. Create endpoint files (create, list, read) in `tugboat-apiserver/src/endpoints/v1_core/`
4. Register handlers in `tugboat-apiserver/src/endpoints/v1_core/mod.rs`

### Configuration

All components use TOML config files. Examples in `sample-configs/`. Default paths: `/etc/tugboat/{component}/config.toml`.
