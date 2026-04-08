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
`tugboat-vm-runtime-interface`, `tugboat-cni-operator`, `tugboat-csi-operator`, `tugboat-scheduler`,
`tugboat-controller-manager`

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
  ├─ tugboat-cni-operator (CNI networking)
  └─ tugboat-csi-operator (CSI storage)

tugboat-controller-manager (workload + storage controllers)
  ├─ tugboat-client
  ├─ tugboat-csi-operator
  └─ tugboat-resources

tugboat-scheduler (ship scheduler)
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

API groups and their resources:
- **core/v1**: Ship, ShipClass, Node, Namespace, PersistentVolume, PersistentVolumeClaim,
  NetworkClass, ClusterNetworkClass, Secret, RuntimeClass, StorageClass, ConfigMap
- **apps/v1**: Deployment, ReplicaSet, Fleet
- **coordination/v1**: Lease

Resources implement traits via the `apply_resource!` macro in `tugboat-resources/src/manifests/mod.rs`:
- `StaticResource` – group, version, kind, plural, singular
- `ClusterScopedResource` or `NamespacedResource` – scope marker
- `ObjectMetaResource` – metadata accessor
- `SetTypeMeta` – type meta setter

Validators are applied via `apply_validators!` macro (e.g., `NameValidator`, `NamespaceProhibitedValidator`).

**Kubernetes concept mapping:** Pod→Ship, Deployment→Deployment, ReplicaSet→ReplicaSet, DaemonSet→Fleet, Container image→VM image (OCI), Dockerfile→Imagefile

### API Server (tugboat-apiserver)

Endpoints live in `tugboat-apiserver/src/endpoints/` organized by API group:
- `v1_core/` – core/v1 resources
- `v1_apps/` – apps/v1 resources (Deployment, ReplicaSet, Fleet)
- `v1_coordination/` – coordination/v1 resources (Lease)

Each resource has separate files for create, list, read, and other operations.
All resources are registered centrally in `endpoints/resource_registry.rs`, which wires routes and exposes discovery verbs.

Key patterns:
- **Cluster-scoped resources** (ShipClass, Namespace, Node, PersistentVolume, ClusterNetworkClass, RuntimeClass, StorageClass): route pattern `/v1/{plural}` and `/v1/{plural}/{name}`
- **Namespaced resources** (Ship, PersistentVolumeClaim, Secret, NetworkClass, ConfigMap, Lease, Deployment, ReplicaSet, Fleet): route pattern `/v1/namespaces/{namespace}/{plural}` and `/v1/namespaces/{namespace}/{plural}/{name}`, plus `/v1/{plural}` for list-all
- Macros in `endpoints/utils.rs`: `extract_object_meta!`, `check_namespace_absent!`, `create_object!`
- `ApiOperator` (in `operator.rs`) wraps `ResourceStore` + `NameGenerator`

### Controller Manager (tugboat-controller-manager)

Runs multiple reconciliation controllers as concurrent tasks:
- **NetworkClassStatusController** – propagates CNI plugin readiness from Nodes to NetworkClass status
- **PvcProvisionerController** – provisions CSI-backed PersistentVolumes for PVCs
- **FleetController** – manages Fleet workloads (DaemonSet-equivalent), creates ReplicaSets per component
- **DeploymentController** – manages Deployment rollouts via ReplicaSets
- **ReplicaSetController** – manages individual Ship replicas for a ReplicaSet
- **PersistentVolumeCleanupController** – deletes CSI-backed PVs when released

Config path: `/etc/tugboat/controller-manager/config.toml`

### Adding a New API Resource

1. Define protobuf in `tugboat-resources/proto/{group}/v1/` and add to `build.rs` compile list
2. Register with `apply_resource!` and `apply_validators!` in `tugboat-resources/src/manifests/mod.rs`
3. Create endpoint files (create, list, read, etc.) in `tugboat-apiserver/src/endpoints/v1_{group}/`
4. Register the resource in `tugboat-apiserver/src/endpoints/resource_registry.rs`
5. Add the resource type to `tugboat-resource-store/src/serializer/mod.rs` via `protobuf_serializable!`

### Configuration

All components use TOML config files. Examples in `sample-configs/`. Default paths:
- `/etc/tugboat/apiserver/config.toml`
- `/etc/tugboat/agent/config.toml`
- `/etc/tugboat/scheduler/config.toml`
- `/etc/tugboat/controller-manager/config.toml`
- `/etc/tugboat/runtime/config.toml`

### Editing guidance

- Recommended: Keep file length to about 800 lines maximum. Very large files are harder to review and understand.
- Recommended (not mandatory): Keep individual functions to approximately 100 lines or less when possible. Prefer splitting complex logic into smaller functions to improve readability and testability.

Note: The function-size guideline is a recommendation, not a strict rule — apply it flexibly based on context.

