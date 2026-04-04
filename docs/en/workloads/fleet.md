# Fleet

A `Fleet` is a high-level abstraction designed to manage a group of related but potentially heterogeneous components as a single unit. It is particularly useful for complex applications that consist of multiple types of VMs that need to communicate with each other.

## Key Features

### Component Management

A `Fleet` allows you to define multiple `components`, each with its own replica count and `Ship` template. The `Fleet` controller automatically creates and manages a `ReplicaSet` for each component.

### Shared Networking

One of the most powerful features of `Fleet` is automatic shared networking. By specifying a `networkClassName` in the `Fleet` spec, Tugboat automatically injects this `NetworkClass` into every `Ship` managed by the `Fleet`. This ensures that all VMs within the `Fleet` are connected to the same private network and can communicate with each other easily.

## Manifest Example

```yaml
apiVersion: apps/v1
kind: Fleet
metadata:
  name: my-app-stack
  namespace: default
spec:
  networkClassName: app-private-net
  components:
    - name: frontend
      replicas: 2
      shipTemplate:
        metadata:
          labels:
            component: frontend
        spec:
          shipClassName: small
          image: "my-registry.local/frontend:v1"
    - name: backend
      replicas: 2
      shipTemplate:
        metadata:
          labels:
            component: backend
        spec:
          shipClassName: medium
          image: "my-registry.local/backend:v1"
```

## Update Behavior

When a `Fleet`'s component template is updated, the `Fleet` controller performs an **in-place update** by directly updating the template of the corresponding `ReplicaSet`. The `ReplicaSet` controller then handles the update of individual `Ship`s. Unlike `Deployment`, `Fleet` does not currently use `ReplicaSet` rotation for updates.

## Status

The `FleetStatus` tracks:
- `totalComponents`: The total number of components defined in the `Fleet`.
- `readyComponents`: The number of components where all desired replicas are ready.
