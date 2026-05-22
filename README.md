# Tugboat

[日本語](./README_ja.md)

Tugboat is a system for orchestrating virtual machines in a Kubernetes‑like manner.

## Introduction

Tugboat is a VM orchestration tool that provides Kubernetes‑style functionality while avoiding the heaviness of KubeVirt
and the complexity of OpenStack.

It aims to manage VMs declaratively—similar to Kubernetes—using a minimal set of components: etcd, tugboat‑apiserver,
tugboat‑agent, tugboat‑runtime, tugboat‑scheduler, and tugboat‑controller‑manager.

## Why Tugboat?

Existing VM orchestration systems come with significant challenges:

### KubeVirt

- Heavy due to running VMs on top of Kubernetes (double layering)
- Complex CRDs and libvirt integration

### OpenStack

- Too many components for individuals or small teams
- Very high learning cost
- Operationally difficult
  Tugboat solves these problems by:
- Inheriting Kubernetes design principles
- Not depending on Kubernetes itself
- Providing a lightweight architecture optimized for VMs
- Remaining simple enough for individuals to run
- Scaling to large deployments through a clean, minimal design

## Key Features

- Kubernetes‑compatible API manifests
    - Uses familiar structures such as `TypeMeta` and `ObjectMeta`
    - Can use `kubectl`
- Declarative cluster powered by etcd
    - The apiserver is stateless; etcd is the single source of truth
- Lightweight control plane
    - Only the apiserver and scheduler are required
- Direct QEMU execution
    - No libvirt; QEMU is invoked directly
- Clear separation of agent and runtime responsibilities
    - Similar to Kubernetes’ kubelet/runtime model
- VM images as OCI artifacts
    - `Imagefile -> build -> push to registry -> referenced by Ship`
    - Supports `qcow2` and `raw` boot disk formats
- CNI support
    - `NetworkClass` / `ClusterNetworkClass` based network configuration
    - agent publishes plugin readiness to `Node.status.cniPlugins`
    - scheduler filters nodes with `NetworkFit`
- RBAC / ServiceAccount
    - `Role` / `ClusterRole`, `RoleBinding` / `ClusterRoleBinding`, `ServiceAccount` API (`authorization/v1`)
    - Fine-grained verb- and resource-level access control enforced in the apiserver
    - Built-in roles: `cluster-admin`, `admin`, `edit`, `view`
    - Opaque bearer tokens generated per ServiceAccount, stored as Secrets
- CustomResourceDefinition (CRD) support
    - User-defined resources with Kubernetes-like `CustomResourceDefinition`
    - OpenAPI v3 schema validation, Namespaced/Cluster scope, and optional `/status` subresource
- High availability design
    - Apiserver can scale horizontally
    - Scheduler uses Lease‑based leader election

## Architecture Overview

![architecture overview](./docs/images/tugboat-structure.svg)

The current crate and extension boundaries are documented in
[docs/architecture.md](./docs/architecture.md). Contributor-facing resource registration steps are documented in
[docs/resource-registration.md](./docs/resource-registration.md).
VM image build, format, runtime support, and cache layout details are documented in
[docs/vm-images.md](./docs/vm-images.md).

### Mapping to Kubernetes Concepts

|   Kubernetes    |    Tugboat     |
|:---------------:|:--------------:|
|       Pod       |      Ship      |
|   ReplicaSet    |   ReplicaSet   |
|   Deployment    |   Deployment   |
|      Node       |      Node      |
| Container image | VM image (OCI) |
|   Dockerfile    |   Imagefile    |
|     kubelet     |     agent      |
|  RuntimeClass   |  RuntimeClass  |

> **Note:** `Fleet` is a Tugboat-specific resource for grouping multiple Ship types that share a private network — it
> has no direct Kubernetes equivalent.

## Manifest Examples

### ShipClass

Defines a VM machine type.
This is a cluster‑scoped resource.

```yaml
apiVersion: v1
kind: ShipClass
metadata:
  name: lightweight
spec:
  cpu:
    architecture: x64
    cores: 2
  memory:
    size: 4Gi
```

### Ship

Defines a VM instance.
This is a namespaced resource.

```yaml
apiVersion: v1
kind: Ship
metadata:
  namespace: default
  name: ship
spec:
  image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
  shipClass: lightweight
  volumes:
    - name: data-disk
      persistentVolumeClaim:
        claimName: data-disk
    - name: app-config
      configMap:
        name: app-config
    - name: app-secret
      secret:
        secretName: app-secret
```

### Fleet

Groups multiple Ship types that share a private network.
This is a namespaced resource (`apps/v1`).

```yaml
apiVersion: apps/v1
kind: Fleet
metadata:
  namespace: default
  name: my-fleet
spec:
  networkClassName: my-network-class
  components:
    - name: frontend
      replicas: 2
      shipTemplate:
        metadata:
          labels:
            role: frontend
        spec:
          image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
          shipClass: lightweight
    - name: backend
      replicas: 3
      shipTemplate:
        metadata:
          labels:
            role: backend
        spec:
          image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
          shipClass: lightweight
```

### CustomResourceDefinition

Tugboat supports user-defined resources via CustomResourceDefinitions, similar to Kubernetes. A CRD declares the resource group, version, kind, plural name, scope, OpenAPI v3 schema, and optional status subresource.

```yaml
apiVersion: apiextensions/v1
kind: CustomResourceDefinition
metadata:
  name: databases.example.com
spec:
  group: example.com
  names:
    plural: databases
    singular: database
    kind: Database
    listKind: DatabaseList
  scope: Namespaced
  versions:
    - name: v1
      served: true
      storage: true
      schema:
        openApiV3Schema: |
          {"type":"object","required":["spec"],"properties":{"spec":{"type":"object"}}}
      subresources:
        status: {}
```

See [docs/custom-resources.md](./docs/custom-resources.md) for the full guide.

### Deployment

Manages a set of identical Ships with rolling-update support.
This is a namespaced resource (`apps/v1`).

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  namespace: default
  name: my-deployment
spec:
  replicas: 3
  selector:
    app: my-app
  shipTemplate:
    metadata:
      labels:
        app: my-app
    spec:
      image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
      shipClass: lightweight
```

### ReplicaSet

Maintains a stable set of replica Ships.
This is a namespaced resource (`apps/v1`).

```yaml
apiVersion: apps/v1
kind: ReplicaSet
metadata:
  namespace: default
  name: my-replicaset
spec:
  replicas: 2
  selector:
    app: my-app
  shipTemplate:
    metadata:
      labels:
        app: my-app
    spec:
      image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
      shipClass: lightweight
```

### RuntimeClass

Declares the capabilities of the VM runtime on a node (live migration support, hotplug support).
This is a cluster‑scoped resource.

```yaml
apiVersion: v1
kind: RuntimeClass
metadata:
  name: standard
spec:
  liveMigration: true
  hotplug:
    cpu:
      add: true
      remove: false
    memory:
      add: true
      remove: false
    nic:
      add: true
      remove: true
    storage:
      add: true
      remove: true
```

A Ship can reference a `RuntimeClass` by name via `spec.runtimeClass`. The scheduler uses the
`RuntimeClassFit` plugin to ensure Ships are only placed on nodes whose associated `RuntimeClass`
satisfies the Ship's requirements (e.g., live migration capability).

### CSI support

CSI-backed volumes can be referenced through `spec.volumes[].persistentVolumeClaim`. The
legacy `volumeClaimRef` field is still accepted for backward compatibility.

Current node-side support matrix:

- [x] `Block` volumeMode
- [x] `Filesystem` volumeMode
- [x] drivers that require `NodeStageVolume` / `NodeUnstageVolume`
- [x] `nodePublishSecretRef` / `nodeStageSecretRef`
- [x] agent restart recovery from persisted publish state
- [x] explicit `fs_type` and `volume_attributes`
- [x] `NodeExpandVolume` / volume expansion
- [x] drivers that require controller publish context
- [x] `NodeGetVolumeStats` / CSI volume health + usage surfacing on PV/PVC conditions

`Filesystem` volumes are exposed to the guest as a 9p share. The mount tag is the Ship
volume name, so both CSI `Filesystem` claims and projected `ConfigMap` / `Secret`
volumes are passed to the guest through the same mechanism.

Control-plane storage support includes `PersistentVolume`, `PersistentVolumeClaim`, and `StorageClass` APIs plus dynamic
CSI provisioning, managed PV cleanup, capacity-aware provisioning/expansion, filesystem claims, and CSI secret /
`fsType` propagation in `tugboat-controller-manager`. Node-side support also includes controller-publish-context
handling, live `NodeExpandVolume` (without Ship recreate) when the driver advertises it, and `NodeGetVolumeStats`-backed
PV/PVC condition updates for CSI health and usage. The main remaining gap is richer recovery beyond persisted publish state.

### Topology-aware scheduling

Tugboat supports topology-aware scheduling to ensure Ships are placed on nodes that have access to the required
resources, such as storage. This is achieved through well-known labels like `topology.tugboat.cloud/region` and
`topology.tugboat.cloud/zone`.

Example manifest for a Ship with node affinity:

```yaml
apiVersion: v1
kind: Ship
metadata:
  name: topology-aware-ship
spec:
  image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
  shipClass: lightweight
  affinity:
    nodeAffinity:
      requiredDuringSchedulingIgnoredDuringExecution:
        nodeSelectorTerms:
          - matchExpressions:
              - key: topology.tugboat.cloud/zone
                operator: In
                values:
                  - zone-a
```

See [topology-aware scheduling](docs/topology-aware-scheduling.md) for more details.

### Volume snapshots and clones

Volume snapshots allow you to create a point-in-time copy of a volume. These snapshots can then be used to
provision new volumes (restore) or to create clones of existing volumes.

Example `VolumeSnapshot` manifest:

```yaml
apiVersion: snapshot/v1
kind: VolumeSnapshot
metadata:
  name: my-snapshot
  namespace: default
spec:
  volumeSnapshotClassName: csi-hostpath-snapclass
  source:
    persistentVolumeClaimName: data-disk
```

See [volume snapshots](docs/volume-snapshots.md) for more details.

### Ship snapshots

Ship snapshots capture the entire state of a VM, including its memory and the state of its volumes. This
allows for full backup and restoration of a VM instance.

Example `ShipSnapshot` manifest:

```yaml
apiVersion: snapshot/v1
kind: ShipSnapshot
metadata:
  name: my-ship-snapshot
  namespace: default
spec:
  shipName: my-ship
  includeVolumes: true
```

See [ship snapshots](docs/ship-snapshots.md) for more details.

### CNI status and Flannel validation

Tugboat now exposes observed CNI readiness through `Node.status.cniPlugins` and
publishes `NetworkClass.status.readyNodes` / `ClusterNetworkClass.status.readyNodes`
from `tugboat-controller-manager`. The scheduler's `NetworkFit` filter uses that
status to reject nodes that do not advertise the plugins required by a Ship's
requested `NetworkClass` / `ClusterNetworkClass`.

The current rollout assumes Flannel itself is installed and managed externally.
For a manual multi-node validation flow:

1. Install the required CNI binaries (`bridge`, `loopback`, `flannel`, and `portmap`
   when port mappings are enabled) on each node under the configured CNI bin directory.
2. Bring up Flannel externally so each node has the expected runtime state
   (by default `/run/flannel/subnet.env` and `/var/lib/cni/flannel`).
3. Start `tugboat-agent` on each node and confirm `kubectl get node -o yaml`
   shows `status.cniPlugins` with the expected readiness.
4. Apply a `ClusterNetworkClass` or `NetworkClass` using `cniPlugin: flannel`
   and confirm its `status.readyNodes` contains the nodes that passed the probe.
5. Create Ships that reference that network class and verify they schedule only to
   ready nodes before performing cross-node connectivity checks.

## RBAC

Tugboat provides a RBAC system.
Access control is enforced in the apiserver for every request.

Detailed documentation:
- [RBAC Overview](./docs/rbac.md)
- [Service Account Tokens & Projection](./docs/service-account-tokens.md)
- [OIDC Integration](./docs/oidc.md)
- [Aggregated ClusterRoles](./docs/aggregated-clusterroles.md)
- [Audit Logging](./docs/audit-logging.md)

### Resources

| Resource             | API Group          | Scope      | Description                                                   |
|----------------------|--------------------|------------|---------------------------------------------------------------|
| `ServiceAccount`     | `core/v1`          | Namespaced | Identity for automated processes and controllers              |
| `Role`               | `authorization/v1` | Namespaced | Permission rules scoped to a single namespace                 |
| `ClusterRole`        | `authorization/v1` | Cluster    | Permission rules that apply cluster-wide                      |
| `RoleBinding`        | `authorization/v1` | Namespaced | Bind a `Role` or `ClusterRole` to subjects within a namespace |
| `ClusterRoleBinding` | `authorization/v1` | Cluster    | Bind a `ClusterRole` to subjects cluster-wide                 |

### Verbs and subject types

Supported verbs: `get`, `list`, `watch`, `create`, `update`, `patch`, `delete`, `deletecollection`

Subject types: `User`, `Group`, `ServiceAccount`

Permission can be further narrowed to specific resource instances via `resourceNames`.

### Built-in ClusterRoles

| ClusterRole     | Description                                                                        |
|-----------------|------------------------------------------------------------------------------------|
| `cluster-admin` | Full access to all resources                                                       |
| `admin`         | Full access within a namespace; cannot modify RBAC or namespace itself             |
| `edit`          | Read/write access to most namespaced resources; cannot read Secrets or modify RBAC |
| `view`          | Read-only access to most namespaced resources                                      |

### Manifest examples

#### ServiceAccount

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  namespace: default
  name: my-service-account
```

#### Role

Grants read access to Ships within the `default` namespace.

```yaml
apiVersion: authorization/v1
kind: Role
metadata:
  namespace: default
  name: ship-reader
rules:
  - apiGroups: [ "" ]
    resources: [ "ships" ]
    verbs: [ "get", "list", "watch" ]
```

#### ClusterRole

Grants read access to Nodes cluster-wide.

```yaml
apiVersion: authorization/v1
kind: ClusterRole
metadata:
  name: node-reader
rules:
  - apiGroups: [ "" ]
    resources: [ "nodes" ]
    verbs: [ "get", "list", "watch" ]
```

#### RoleBinding

Binds `ship-reader` to a user and a ServiceAccount within the `default` namespace.

```yaml
apiVersion: authorization/v1
kind: RoleBinding
metadata:
  namespace: default
  name: ship-reader-binding
roleRef:
  apiGroup: authorization
  kind: Role
  name: ship-reader
subjects:
  - kind: User
    name: alice
  - kind: ServiceAccount
    namespace: default
    name: my-service-account
```

#### ClusterRoleBinding

Grants the `node-reader` ClusterRole to an entire group cluster-wide.

```yaml
apiVersion: authorization/v1
kind: ClusterRoleBinding
metadata:
  name: node-reader-binding
roleRef:
  apiGroup: authorization
  kind: ClusterRole
  name: node-reader
subjects:
  - kind: Group
    name: ops-team
```

#### Aggregated ClusterRole

Extends the `view` role with custom resource permissions.

```yaml
apiVersion: authorization/v1
kind: ClusterRole
metadata:
  name: my-extension-view
  labels:
    rbac.tugboat.cloud/aggregate-to-view: "true"
rules:
  - apiGroups: ["my.example.com"]
    resources: ["myresources"]
    verbs: ["get", "list", "watch"]
```

#### OIDC RoleBinding

```yaml
apiVersion: authorization/v1
kind: RoleBinding
metadata:
  name: oidc-developers-binding
  namespace: default
subjects:
  - kind: Group
    name: oidc:developers
    apiGroup: authorization/v1
    roleRef:
      kind: ClusterRole
      name: view
      apiGroup: authorization/v1
```

#### Ship with projected token

```yaml
apiVersion: core/v1
kind: Ship
metadata:
  name: my-ship
spec:
  serviceAccountName: my-sa
  automountServiceAccountToken: true
```

## Roadmap

- [x] Runtime
    - [x] tugboat-qemu-runtime
    - [x] tugboat-cloud-hypervisor-runtime
- [x] tugboat-resources (Resource definitions)
- [x] tugboat-resource-store (etcd wrapper for apiserver)
- [x] tugboat‑apiserver
- [x] tugboat-client
- [x] tugboat-cli build (Build a VM Image from a Imagefile)
- [x] fieldSelector and labelSelector
- [x] tugboat‑scheduler
- [x] tugboat‑agent
    - [x] Node auto-registration
    - [x] Reconcile on Ship Added events
    - [x] Networking (CNI, NetworkClass / ClusterNetworkClass)
    - [x] Reconcile on Ship Modified events
    - [x] Reconcile on Ship Deleted events
    - [x] Storage (CSI publish/stage, controller publish context, and live expansion)
    - [x] Topology-aware scheduling and snapshot-style workflows
- [x] Secret
- [x] Namespace resource definition and API (`core/v1`)
- [x] tugboat-controller-manager
    - [x] Dynamic CSI volume provisioning
    - [x] CSI-backed managed PV cleanup
    - [x] ReplicaSet controller (maintaining the prescribed number of Ships)
    - [x] Deployment controller (rolling-update management of ReplicaSets)
    - [x] Fleet controller
    - [x] NamespaceDefaultServiceAccountController (auto-creates `default` ServiceAccount per Namespace)
    - [x] ServiceAccount token issuance controller (creates `service-account-token` Secrets)
- [x] Fleet resource definition and API (`apps/v1`)
- [x] ReplicaSet resource definition and API (`apps/v1`)
- [x] Deployment resource definition and API (`apps/v1`)
- [x] ConfigMap
- [x] NetworkClass / ClusterNetworkClass resource definition and API (`core/v1`)
- [x] PersistentVolume / PersistentVolumeClaim / StorageClass resource definition and API (`core/v1`)
- [x] Lease resource definition and API (`coordination/v1`)
- [x] Live migration
    - [x] Core migration triggered by `target_node_name`
    - [x] Migration state machine (Pending, Ready, Migrating, Completed, Failed)
    - [x] Migration status and conditions reflected on Ship
    - [x] Preflight compatibility checks (CPU, shared-storage eligibility, target network capability)
    - [x] Reliable recovery and explicit error reporting on migration failure
    - [x] Guest/network continuity via deterministic bridge/interface/MAC identity across nodes
    - [x] Timeout detection with automatic QEMU cancel (Pending: 2 min, Migrating: 30 min)
    - [x] Per-ShipClass configurable QEMU migration parameters (bandwidth, downtime, xbzrle cache, postcopy)
    - [x] Scheduler StorageFit plugin rejects nodes for Ships with non-RWX volumes
- [x] RuntimeClass
    - [x] Resource definition and API (`core/v1`)
    - [x] `spec.runtimeClass` field on Ship
    - [x] Scheduler `RuntimeClassFit` plugin (live migration capability check)
    - [x] Hotplug operations gated by RuntimeClass flags
- [x] RBAC / ServiceAccount
    - [x] `ServiceAccount` resource definition and API (`core/v1`)
    - [x] `Role` / `ClusterRole` resource definition and API (`authorization/v1`)
    - [x] `RoleBinding` / `ClusterRoleBinding` resource definition and API (`authorization/v1`)
    - [x] RBAC authorization enforcement in apiserver
    - [x] Built-in roles (`cluster-admin`, `admin`, `edit`, `view`)
    - [x] ServiceAccount token generation and validation
        - [x] Opaque bearer token issued via Secret of type `service-account-token`
        - [x] Default ServiceAccount auto-created per namespace
        - [x] Signed JWT tokens with audience/expiry
    - [x] ServiceAccount token auto-projection into Ships
    - [x] OIDC integration for external identity providers
    - [x] Aggregated ClusterRoles
    - [x] Audit logging
- [x] Installer (systemd setup)
    - [x] `install-control-plane.sh` — installs etcd, apiserver, scheduler, controller-manager as systemd units
    - [x] `install-worker.sh` — installs agent and VM runtime (QEMU or Cloud Hypervisor) as systemd units
    - [x] TLS / PKI auto-generation (`setup-pki.sh`, `setup-etcd-pki.sh`)
    - [x] RBAC bootstrap for control-plane ServiceAccount tokens (`bootstrap-rbac.sh`)
    - [x] Flannel CNI support (static subnet.env / vxlan / host-gw via `bootstrap-flannel.sh`)
    - [x] CSI hostpath provisioner installer (`install-csi-hostpath.sh`)
    - [x] `uninstall.sh` — stops and removes all Tugboat units, binaries, and configs
    - [x] Docker-based isolated test environment with scenario scripts (`installer/systemd/test/`)
- [x] Installer (Ansible)
    - [x] Inventory-driven control-plane, worker, and CSI hostpath roles
    - [x] Secure-mode PKI and ServiceAccount material distribution
    - [x] Idempotency markers and service health checks around the systemd installers
    - [x] `site.yml` and `uninstall.yml` playbooks with Docker scenario tests
- [x] Snapshot
    - [x] `VolumeSnapshot`, `VolumeSnapshotContent`, and `VolumeSnapshotClass` resource definitions and API (`snapshot/v1`)
    - [x] `ShipSnapshot` resource definition and API (`snapshot/v1`)
    - [x] VolumeSnapshot controller for CSI snapshot create/delete and status propagation
    - [x] ShipSnapshot volume fanout and agent runtime snapshot reconciliation
    - [x] Runtime snapshot create/delete/restore/list support for QEMU and Cloud Hypervisor
    - [x] PVC restore and clone via `spec.dataSource`
- [x] CRD
- [ ] Production readiness
    - [ ] Observability
        - [ ] Add `/readyz` / `/livez` endpoints for control-plane components and node agents
        - [ ] Expose Prometheus-style metrics for apiserver, scheduler, controller-manager, agent, CNI, CSI, and runtimes
        - [ ] Provide default dashboards and alerting examples for scheduling, reconciliation, storage, networking, runtime, and audit failures
    - [ ] Control-plane HA and failure handling
        - [ ] Add Lease-based leader election or another single-writer mechanism for controller-manager controllers
        - [ ] Define resync, retry, backoff, and stuck-reconcile recovery behavior for every controller and agent operation
        - [ ] Document load-balancer and multi-apiserver deployment patterns with readiness-aware routing
    - [ ] Security hardening
        - [ ] Support certificate and ServiceAccount signing-key rotation without cluster downtime
        - [ ] Add encryption-at-rest support for Secrets and other sensitive resources stored in etcd
        - [ ] Provide production config profiles that disable insecure HTTP, insecure etcd, anonymous auth, and development-only defaults
        - [ ] Finish audit log size-based rotation and age-based retention; surface dropped audit events as metrics
    - [ ] Backup and disaster recovery
        - [ ] Document and test etcd snapshot/restore procedures
        - [ ] Add scheduled backup/retention workflows for ShipSnapshot and VolumeSnapshot resources
        - [ ] Validate restore drills for control-plane state, VM runtime state, and CSI-backed persistent volumes
    - [ ] Upgrade and release operations
        - [ ] Define rolling-upgrade procedures and compatibility guarantees across apiserver, controllers, scheduler, agents, and runtimes
        - [ ] Add API version migration/conversion strategy for future breaking resource changes
        - [ ] Publish signed release artifacts, SBOMs, and reproducible install inputs
    - [ ] Production networking and storage validation
        - [ ] Manage or fully document multi-node CNI lifecycle instead of relying on externally managed Flannel state
        - [ ] Maintain a tested CSI driver compatibility matrix covering topology, expansion, snapshots, clones, health, and recovery
        - [ ] Add or document in-place attach/detach behavior for volumes and filesystem shares, including virtiofs migration from 9p
    - [ ] Installer and operational tooling
        - [ ] Add idempotent upgrade, rollback, and config-drift checks to the systemd installer
        - [ ] Add host preflight checks for KVM, QEMU/Cloud Hypervisor, CNI binaries, CSI sockets, kernel modules, disk space, and time sync
        - [ ] Extend isolated installer scenarios into a release-blocking multi-node smoke suite

## Installation

Tugboat currently ships two systemd-based installation paths:

- `installer/ansible/` provides inventory-driven playbooks for one or more
  Ubuntu 24.04 hosts. Use this path for repeatable installs across
  control-plane, worker, and CSI hostpath nodes.
- `installer/systemd/` provides the lower-level shell scripts that install and
  configure one host at a time on Debian/Ubuntu.

Detailed guides:

- [Ansible installer guide](./docs/en/installation/ansible.md)
- [systemd installer guide](./docs/en/installation/systemd.md)

### Ansible

Install a single-node or multi-node inventory with:

```bash
cd installer/ansible
ansible-playbook -i inventory.example.yml site.yml
```

The playbooks install the control plane, workers, and CSI hostpath provisioner
based on inventory group membership:

- `tugboat_control_plane`: etcd, apiserver, scheduler, controller-manager
- `tugboat_workers`: agent, VM runtime, CNI plugins, optional Flannel daemon
- `tugboat_csi_hostpath`: CSI hostpath provisioner

Secure mode is enabled by default. The bootstrap role generates PKI on the first
control-plane host, distributes shared control-plane certificates, bootstraps
ServiceAccount/RBAC data, and copies worker CA/token material when workers are
in the same inventory.

Uninstall through the same inventory:

```bash
cd installer/ansible
ansible-playbook -i inventory.example.yml uninstall.yml
```

### systemd scripts

The `installer/systemd/` directory contains shell scripts that install and
configure Tugboat on a Debian/Ubuntu host. Root access and a working `apt-get`
are required.

#### Control plane

```bash
sudo installer/systemd/install-control-plane.sh \
    --build \
    --apiserver-host control-plane.example.com \
    --apiserver-ip 192.168.0.1
```

This installs **etcd**, **tugboat-apiserver**, **tugboat-scheduler**, and
**tugboat-controller-manager** as systemd services.  By default the API server
listens on `0.0.0.0:8443` with TLS (auto-generated CA + server certificate).
Pass `--insecure` to use plain HTTP on port 8080 instead.

Use `--use-prebuilt --bin-dir <path>` in place of `--build` to supply
pre-compiled binaries.

#### Worker

```bash
sudo installer/systemd/install-worker.sh \
    --use-prebuilt --bin-dir /path/to/bins \
    --apiserver-url https://192.168.0.1:8443 \
    --ca-cert /etc/tugboat/pki/ca.crt \
    --service-account-token /path/to/agent-token
```

This installs **tugboat-agent** and the selected VM runtime (`--runtime qemu`
or `--runtime cloud-hypervisor`).  CNI plugins are installed automatically.
Flannel mode is controlled by `--flannel-mode static|vxlan|host-gw` (default:
`static`, which writes `/run/flannel/subnet.env` without running flanneld).

#### Uninstall

```bash
sudo installer/systemd/uninstall.sh --control-plane   # control-plane node
sudo installer/systemd/uninstall.sh --worker          # worker node
sudo installer/systemd/uninstall.sh --control-plane --worker --purge  # full cleanup
```

### Testing the installers

The installer test suites use Docker-based Ubuntu 24.04 systemd containers.
Run the systemd scenarios with:

```bash
installer/systemd/test/run-tests.sh
```

Run the Ansible scenarios with:

```bash
installer/ansible/test/run-tests.sh
```

## Contributing

Tugboat is still in an early stage, and contributions of any kind are welcome.

## License

Apache License 2.0
See [LICENSE](./LICENSE) for details.
