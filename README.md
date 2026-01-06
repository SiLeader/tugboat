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
- Planned CNI support
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
    architecture: x86_64
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

## Roadmap

- [x] tugboat-runtime
- [x] tugboat-resources (Resource definitions)
- [x] tugboat-resource-store (etcd wrapper for apiserver)
- [x] tugboat‑apiserver
- [x] tugboat-client
- [x] tugboat-cli build (Build a VM Image from a Imagefile)
- [x] fieldSelector and labelSelector
- [ ] tugboat‑agent (Work in progress!!)
    - [ ] Networking (CNI)
    - [ ] Storage (CSI)
- [ ] tugboat‑scheduler
- [ ] Fleet
- [ ] Secret / ConfigMap
- [ ] Live migration
- [ ] RBAC / ServiceAccount
- [ ] CRD

## Contributing

Tugboat is still in an early stage, and contributions of any kind are welcome.

## License

Apache License 2.0
See [LICENSE](./LICENSE) for details.
