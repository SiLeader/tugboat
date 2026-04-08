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
    - `Imagefile → build → push to registry → referenced by Ship`
- CNI support
    - `NetworkClass` / `ClusterNetworkClass` based network configuration
    - agent publishes plugin readiness to `Node.status.cniPlugins`
    - scheduler filters nodes with `NetworkFit`
- Planned CRD support
- High availability design
    - Apiserver can scale horizontally
    - Scheduler uses Lease‑based leader election

## Architecture Overview

![architecture overview](./docs/images/tugboat-structure.svg)

### Mapping to Kubernetes Concepts

|   Kubernetes    |     Tugboat     |
|:---------------:|:---------------:|
|       Pod       |      Ship       |
|   ReplicaSet    |   ReplicaSet    |
|   Deployment    |   Deployment    |
|      Node       |      Node       |
| Container image | VM image (OCI)  |
|   Dockerfile    |    Imagefile    |
|     kubelet     |      agent      |
|  RuntimeClass   |  RuntimeClass   |

> **Note:** `Fleet` is a Tugboat-specific resource for grouping multiple Ship types that share a private network — it has no direct Kubernetes equivalent.

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
PV/PVC condition updates for CSI health and usage. The main remaining gaps are scheduler awareness of storage
constraints, richer recovery beyond persisted publish state, and snapshot/clone style workflows.

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
   (by default `/run/flannel/subnet.env` and `/run/flannel`).
3. Start `tugboat-agent` on each node and confirm `kubectl get node -o yaml`
   shows `status.cniPlugins` with the expected readiness.
4. Apply a `ClusterNetworkClass` or `NetworkClass` using `cniPlugin: flannel`
   and confirm its `status.readyNodes` contains the nodes that passed the probe.
5. Create Ships that reference that network class and verify they schedule only to
   ready nodes before performing cross-node connectivity checks.

## Roadmap

- [x] tugboat-runtime
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
    - [ ] Topology-aware scheduling and snapshot-style workflows
- [x] Secret
- [x] tugboat-controller-manager
    - [x] Dynamic CSI volume provisioning
    - [x] CSI-backed managed PV cleanup
    - [x] ReplicaSet controller (maintaining the prescribed number of Ships)
    - [x] Deployment controller (rolling-update management of ReplicaSets)
    - [x] Fleet controller
- [x] Fleet resource definition and API (`apps/v1`)
- [x] ReplicaSet resource definition and API (`apps/v1`)
- [x] Deployment resource definition and API (`apps/v1`)
- [x] ConfigMap
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
- [ ] RBAC / ServiceAccount
- [ ] CRD

## Contributing

Tugboat is still in an early stage, and contributions of any kind are welcome.

## License

Apache License 2.0
See [LICENSE](./LICENSE) for details.
