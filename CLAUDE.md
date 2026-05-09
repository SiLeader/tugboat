# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
# Build entire workspace
cargo build --release

# Build a single package
cargo build --release --package tugboat-apiserver

# Run tests
cargo test --workspace --all-targets
cargo test --package tugboat-resources

# Lint & format
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --check
cargo fmt  # auto-format

# Dependency policy and security audit (uses deny.toml)
cargo deny check
```

### Required Gates

Run the PR gate before merging changes:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo deny check
```

`cargo deny check` is both a PR gate and a release-blocking gate. Release verification also includes the fixed installer
scenario set in `docs/verification-guide.md`.

Packages: `tugboat-resources`, `tugboat-apiserver`, `tugboat-agent`, `tugboat-qemu-runtime`,
`tugboat-cloud-hypervisor-runtime`, `tugboat-resource-store`, `tugboat-client`, `tugboat-cli`,
`tugboat-vm-image`, `tugboat-vm-runtime-interface`, `tugboat-cni-operator`, `tugboat-csi-operator`,
`tugboat-scheduler`, `tugboat-controller-manager`

## Architecture

Tugboat is a Kubernetes-inspired VM orchestration system written in Rust. It manages VMs declaratively
using etcd as the single source of truth. QEMU and Cloud Hypervisor are implemented as runtime command backends.

Use `docs/architecture.md` as the public overview of current crate boundaries and extension points.

### Component Dependency Graph

```
tugboat-apiserver (actix-web REST API)
  ├─ tugboat-resources (with "schema" feature for OpenAPI)
  └─ tugboat-resource-store (etcd CRUD/watch wrapper)

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

tugboat-cni-operator / tugboat-csi-operator (node integration operators)
  └─ external process/protocol boundaries

tugboat-qemu-runtime (QEMU executor)
  ├─ tugboat-resources
  ├─ tugboat-vm-runtime-interface
  └─ tugboat-runtime-common

tugboat-cloud-hypervisor-runtime (Cloud Hypervisor executor)
  ├─ tugboat-resources
  ├─ tugboat-vm-runtime-interface
  └─ tugboat-runtime-common
```

### Resource System (tugboat-resources)

All API types are defined as protobuf in `tugboat-resources/proto/` and compiled via `build.rs`
using prost-build. The build script applies serde + optional utoipa (OpenAPI) derives.

API groups and their resources:

- **core/v1**: Ship, ShipClass, Node, Namespace, PersistentVolume, PersistentVolumeClaim,
  NetworkClass, ClusterNetworkClass, Secret, ServiceAccount, RuntimeClass, StorageClass, ConfigMap
- **apps/v1**: Deployment, ReplicaSet, Fleet
- **authorization/v1**: Role, RoleBinding, ClusterRole, ClusterRoleBinding
- **coordination/v1**: Lease

Resource API metadata lives in `tugboat-resources/src/resource_api.rs`. Generated resource types
implement traits via the `apply_resource!` macro in `tugboat-resources/src/manifests/mod.rs`,
which points at that metadata table:

- `StaticResource` – group, version, kind, plural, singular
- `ClusterScopedResource` or `NamespacedResource` – scope marker
- `ObjectMetaResource` – metadata accessor
- `SetTypeMeta` – type meta setter

Validators are applied via `apply_validators!` macro (e.g., `NameValidator`, `NamespaceProhibitedValidator`).

**Kubernetes concept mapping:** Pod→Ship, Deployment→Deployment, ReplicaSet→ReplicaSet, DaemonSet→Fleet, Container
image→VM image (OCI), Dockerfile→Imagefile

### API Server (tugboat-apiserver)

Endpoints live in `tugboat-apiserver/src/endpoints/` organized by API group:

- `v1_core/` – core/v1 resources
- `v1_apps/` – apps/v1 resources (Deployment, ReplicaSet, Fleet)
- `v1_authorization/` – authorization/v1 RBAC resources
- `v1_coordination/` – coordination/v1 resources (Lease)

Each resource has a thin endpoint wrapper. All resources are registered centrally in
`endpoints/resource_registry.rs`, which wires routes and exposes discovery verbs from the shared resource metadata
descriptors.

Key patterns:

- **Cluster-scoped resources** (ShipClass, Namespace, Node, PersistentVolume, ClusterNetworkClass, RuntimeClass,
  StorageClass): route pattern `/v1/{plural}` and `/v1/{plural}/{name}`
- **Namespaced resources** (Ship, PersistentVolumeClaim, Secret, NetworkClass, ConfigMap, Lease, Deployment, ReplicaSet,
  Fleet): route pattern `/v1/namespaces/{namespace}/{plural}` and `/v1/namespaces/{namespace}/{plural}/{name}`, plus
  `/v1/{plural}` for list-all
- Macros in `endpoints/utils.rs`: `extract_object_meta!`, `check_namespace_absent!`, `create_object!`
- `ApiOperator` (in `operator.rs`) wraps `ResourceStore` + `NameGenerator`

### Controller Manager (tugboat-controller-manager)

Runs multiple reconciliation controllers as concurrent tasks:

- **NetworkClassStatusController** – propagates CNI plugin readiness from Nodes to NetworkClass status
- **PvcProvisionerController** – provisions CSI-backed PersistentVolumes for PVCs
- **FleetController** – manages Fleet workloads (DaemonSet-equivalent), creates ReplicaSets per component
- **DeploymentController** – manages Deployment rollouts via ReplicaSets
- **ReplicaSetController** – manages individual Ship replicas for a ReplicaSet
- **NamespaceDefaultServiceAccountController** – creates the default ServiceAccount in each Namespace
- **ServiceAccountTokenController** – creates service-account-token Secrets
- **PersistentVolumeCleanupController** – deletes CSI-backed PVs when released

Shared controller boundaries:

- `base.rs` and `error.rs` define the controller runtime surface used inside controller-manager
- `workload/mod.rs` contains selector, owner reference, and workload helper logic
- `deployment/rs_ops.rs` and `deployment/template_hash.rs` isolate Deployment-to-ReplicaSet behavior
- `replicaset/scale.rs`, `ship_builder.rs`, `status.rs`, and `template_update.rs` split ReplicaSet responsibilities

Config path: `/etc/tugboat/controller-manager/config.toml`

### Adding a New API Resource

Follow `docs/resource-registration.md`. The short version is:

1. Define protobuf in `tugboat-resources/proto/{group}/v1/` and add it to the `build.rs` compile list
2. Add one descriptor in `tugboat-resources/src/resource_api.rs`
3. Register the generated type with `apply_resource!` and `apply_validators!` in
   `tugboat-resources/src/manifests/mod.rs`
4. Create thin endpoint wrapper files in `tugboat-apiserver/src/endpoints/v1_{group}/`
5. Wire the descriptor to the endpoint wrapper in `tugboat-apiserver/src/endpoints/resource_registry.rs`
6. Add the resource type to `tugboat-resource-store/src/serializer/mod.rs` via `protobuf_serializable!`
7. Add or update descriptor, serializer, discovery, RBAC, and integration tests listed in `docs/resource-registration.md`

### Adding a Controller

Controller-manager controllers implement the local trait in `tugboat-controller-manager/src/base.rs`, return errors
through `error.rs`, and are registered in `tugboat-controller-manager/src/lib.rs`. Workload controllers should reuse
`workload/`, `deployment/rs_ops.rs`, and `replicaset/` helpers before adding new cross-controller helpers.

### Adding a Runtime Command

Runtime command names and JSON payloads are external contracts. Add request/response structs and shared logical
validation in `tugboat-vm-runtime-interface`; put config, path, signal, and process helpers in
`tugboat-runtime-common`; then implement backend-specific loading, planning, API calls, rollback, and reporting in
both runtime crates when the command is supported by both backends.

### Configuration

All components use TOML config files. Examples in `sample-configs/`. Default paths:

- `/etc/tugboat/apiserver/config.toml`
- `/etc/tugboat/agent/config.toml`
- `/etc/tugboat/scheduler/config.toml`
- `/etc/tugboat/controller-manager/config.toml`
- `/etc/tugboat/runtime/config.toml`
- `/etc/tugboat/runtime/cloud-hypervisor-config.toml`

### Editing guidance

- Recommended: Keep file length to about 800 lines maximum. Very large files are harder to review and understand.
- Recommended (not mandatory): Keep individual functions to approximately 100 lines or less when possible. Prefer
  splitting complex logic into smaller functions to improve readability and testability.

Note: The function-size guideline is a recommendation, not a strict rule — apply it flexibly based on context.

The system's domain is `tugboat.cloud`. If you use a domain, please use it consistently.
