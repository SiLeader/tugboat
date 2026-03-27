# Tugboat

[日本語](./docs/README_ja.md)

Tugboat is a system for orchestrating virtual machines in a Kubernetes‑like manner.

## Introduction

Tugboat is a VM orchestration tool that provides Kubernetes‑style functionality while avoiding the heaviness of KubeVirt
and the complexity of OpenStack.

It aims to manage VMs declaratively—similar to Kubernetes—using a minimal set of components: etcd, tugboat‑apiserver,
tugboat‑agent, tugboat‑runtime, and tugboat‑scheduler.

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
|   Deployment    | Fleet (Planned) |
|      Node       |      Node       |
| Container image | VM image (OCI)  |
|   Dockerfile    |    Imagefile    |
|     kubelet     |      agent      |

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
  image: example.com/vm-images/ubuntu:24.04
  shipClass: lightweight
```

### CSI support

CSI-backed volumes are referenced through `volumeClaimRef`.

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

`Filesystem` volumes are exposed to the guest as a 9p share. The mount tag is the referenced `volumeClaimRef[].name`.

Control-plane storage support includes `PersistentVolume`, `PersistentVolumeClaim`, and `StorageClass` APIs plus dynamic CSI provisioning, managed PV cleanup, capacity-aware provisioning/expansion, filesystem claims, and CSI secret / `fsType` propagation in `tugboat-controller-manager`. Node-side support also includes controller-publish-context handling, `NodeExpandVolume` when the driver advertises it, and `NodeGetVolumeStats`-backed PV/PVC condition updates for CSI health and usage. The main remaining gaps are scheduler awareness of storage constraints, richer recovery beyond persisted publish state, and snapshot/clone style workflows.

## Roadmap

- [x] tugboat-runtime
- [x] tugboat-resources (Resource definitions)
- [x] tugboat-resource-store (etcd wrapper for apiserver)
- [x] tugboat‑apiserver
- [x] tugboat-client
- [x] tugboat-cli build (Build a VM Image from a Imagefile)
- [x] fieldSelector and labelSelector
- [x] tugboat‑scheduler
- [ ] tugboat‑agent (In progress)
    - [x] Node auto-registration
    - [x] Reconcile on Ship Added events
    - [x] Networking (CNI, NetworkClass / ClusterNetworkClass)
    - [ ] Reconcile on Ship Modified events
    - [x] Reconcile on Ship Deleted events
    - [ ] Storage (CSI provisioning, publish/stage, controller publish context, and expansion are implemented; topology-aware scheduling and snapshot-style workflows remain)
- [x] Secret
- [ ] tugboat-controller-manager
    - [x] Dynamic CSI volume provisioning
    - [x] CSI-backed managed PV cleanup
    - [ ] ReplicaSet (Maintaining the prescribed number of ships)
    - [ ] Deployment (Deploying same configuration Ships)
    - [ ] Fleet
- [ ] ConfigMap
- [ ] Live migration
- [ ] RBAC / ServiceAccount
- [ ] CRD

## Contributing

Tugboat is still in an early stage, and contributions of any kind are welcome.

## License

Apache License 2.0
See [LICENSE](./LICENSE) for details.
