# Architecture Overview

Tugboat is a Kubernetes-inspired VM orchestration system. The API server stores declarative resources in etcd, controllers reconcile higher-level resources into lower-level resources, the scheduler assigns `Ship`s to `Node`s, and each node agent reconciles assigned `Ship`s through CNI, CSI, and a VM runtime command.

## Crate Boundaries

| Crate | Responsibility |
| --- | --- |
| `tugboat-resources` | Protobuf-generated API types, resource metadata descriptors, manifest traits, and validators. |
| `tugboat-resource-store` | etcd-backed CRUD and watch storage, plus protobuf serialization. |
| `tugboat-apiserver` | Actix Web REST API, discovery, status subresources, authn (JWT, OIDC, Certificates), authz (RBAC), RBAC bootstrap, audit logging, and route registration. |
| `tugboat-client` | HTTP client, watch stream handling, reflector support, and controller runtime modules. |
| `tugboat-controller-manager` | Workload, storage, network status, namespace defaulting, service account token, and snapshot controllers. |
| `tugboat-scheduler` | Scheduling cycle and filter/score plugins (VolumeTopology, NodeAffinity, ShipAffinity, ShipAntiAffinity, TopologySpread, ImageLocality). |
| `tugboat-agent` | Node registration, assigned-`Ship` reconciliation, CNI/CSI orchestration, volume materialization, migration, and runtime operation calls. |
| `tugboat-vm-runtime-interface` | Stable runtime command JSON contract and logical request validation shared by runtime backends and the agent. |
| `tugboat-runtime-common` | Runtime config loading, process/signal helpers, path validation, and common pre-exec setup. |
| `tugboat-qemu-runtime` | QEMU runtime command implementation and QMP-backed VM operations. |
| `tugboat-cloud-hypervisor-runtime` | Cloud Hypervisor runtime command implementation and HTTP API-backed VM operations. |
| `tugboat-cni-operator` | CNI plugin execution and network attachment support. |
| `tugboat-csi-operator` | CSI controller/node calls, retry classification, and storage operation support. |
| `tugboat-cli` / `tugboat-vm-image` | User-facing CLI and OCI VM image build/pull/push tooling. |

## Control Plane Flow

```
client / CLI
  -> tugboat-apiserver
     -> tugboat-resource-store
        -> etcd

tugboat-controller-manager
  -> tugboat-client watch/list APIs
  -> creates or patches derived resources

tugboat-scheduler
  -> watches unscheduled Ships
  -> patches spec.nodeName

tugboat-agent
  -> watches Ships assigned to its Node
  -> resolves image, network, storage, and secrets
  -> calls runtime commands through tugboat-vm-runtime-interface
```

The API contract is resource-oriented. Resource group/version/kind/plural, discovery verbs, RBAC resource strings, manifest shape, runtime command names, and runtime JSON payload semantics are external contracts.

## API Resource Boundary

Resource metadata is centralized in `tugboat-resources/src/resource_api.rs`. The same descriptor data is consumed by:

- `StaticResource` implementations in `tugboat-resources/src/manifests/mod.rs`
- apiserver registration in `tugboat-apiserver/src/endpoints/resource_registry.rs`
- discovery and RBAC verb/resource generation
- serializer coverage tests in `tugboat-resource-store`

Endpoint files remain per resource under `tugboat-apiserver/src/endpoints/v1_{group}/`, but the route registry is descriptor-driven. Adding a new resource should follow [resource-registration.md](./resource-registration.md).

### API Groups

The following API groups and versions are currently supported:

- `core/v1`: Core resources (Ship, Node, Secret, ConfigMap, etc.)
- `apps/v1`: Workload resources (Deployment, ReplicaSet, Fleet)
- `authorization/v1`: RBAC resources (Role, ClusterRole, etc.)
- `coordination/v1`: Lease resources
- `snapshot.tugboat.cloud/v1`: Snapshot resources (VolumeSnapshot, VolumeSnapshotContent, VolumeSnapshotClass, ShipSnapshot)

## Controller Boundary

Controller-manager controllers share the `base.rs` controller trait and `error.rs` error handling. Workload-specific common logic lives under:

- `workload/mod.rs`: selector, owner reference, template hash helpers, and shared workload decisions
- `deployment/rs_ops.rs`: Deployment-to-ReplicaSet operations
- `deployment/template_hash.rs`: compatibility-sensitive template hashing
- `replicaset/scale.rs`: replica count decisions
- `replicaset/ship_builder.rs`: `Ship` construction
- `replicaset/status.rs`: observed state aggregation
- `replicaset/template_update.rs`: template update and hotplug-aware decisions

Controllers with external side effects remain explicit modules: `pvc_provisioner.rs`, `pv_cleanup.rs`, `network_class_status.rs`, `namespace_default_service_account.rs`, `service_account_token_controller.rs`, `aggregated_clusterrole.rs`, `volume_snapshot_controller.rs`, and `ship_snapshot_volumes_controller.rs`.


## Agent Boundary

The agent reconciler is organized around operation entry points under `tugboat-agent/src/reconciler/ops/`:

- `add.rs` with `add/recovery.rs`: image, volume, network, runtime create/start, and recovery from partial state
- `modify.rs` with `modify/plan.rs`: spec diffing and modify execution
- `migration.rs` with `migration/preflight.rs`: migration state transitions, source/target flow, timeout handling, and preflight checks
- `hotplug.rs`: agent-side hotplug execution using runtime-interface validation
- `snapshot.rs`: runtime snapshot and volume snapshot capture
- `delete.rs`: runtime, network, and storage cleanup
- `secret_resolver.rs` and `volume_provisioner.rs`: support boundaries for storage and secret materialization

Volume normalization and materialization are separated into `reconciler/volume/` and `reconciler/materialized_volume.rs`. Runtime command calls are wrapped under `tugboat-agent/src/runtime/`.

## Runtime Boundary

Runtime commands are stable CLI subcommands:

- `create`
- `start`
- `status`
- `stop`
- `hotplug`
- `migrate`
- `migration-status`
- `migrate-cancel`
- `run`

The request/response structs live in `tugboat-vm-runtime-interface`. Logical validation that should be identical across backends belongs there, for example hotplug ID normalization, safe IDs, absolute block volume paths, and valid hotplug volume kind/format. Backend-specific constraints, QMP details, Cloud Hypervisor HTTP details, and rollback behavior stay in the backend crates.

Each runtime command should keep this shape:

1. Load JSON or CLI input.
2. Validate through `tugboat-vm-runtime-interface` and `tugboat-runtime-common` where applicable.
3. Build a backend-specific plan or API request.
4. Apply the operation through the backend adapter.
5. Report structured success or `tugboat_vm_runtime_interface::error::RuntimeError`.

## Verification

Use [verification-guide.md](./verification-guide.md) for gate commands. The short policy is:

- PR gate: format, clippy, workspace tests, and `cargo deny check`
- Component gate: run the package and integration tests that cover the touched boundary
- Phase completion gate: PR gate plus relevant component gates and the control-plane installer smoke test
- Release gate: PR gate, `cargo deny check` as a release blocker, and the fixed installer scenario set

