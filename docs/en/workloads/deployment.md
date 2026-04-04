# Deployment

A `Deployment` provides declarative updates for `Ship`s and `ReplicaSet`s. You describe a desired state in a `Deployment`, and the `Deployment` controller changes the actual state to the desired state at a controlled rate.

## How it works

A `Deployment` manages one or more `ReplicaSet`s. When you update the `Ship` template in a `Deployment`, it creates a new `ReplicaSet` and gradually moves `Ship`s from the old `ReplicaSet` to the new one, or updates the existing `ReplicaSet` depending on the change and the strategy.

Tugboat distinguishes between two update paths:
1. **Rotation Path**: Used for changes that require a VM restart or replacement (e.g., changing the VM image).
2. **In-place Path**: Used for changes that can be applied to a running VM (e.g., changing `shipClass` if supported by the agent via hotplug).

## Update Strategies

### RollingUpdate (Default)

The `Deployment` replaces the old `ReplicaSet` with the new one by gradually scaling up the new one and scaling down the old one.

- `maxSurge`: The maximum number of `Ship`s that can be created over the desired number of `Ship`s.
- `maxUnavailable`: The maximum number of `Ship`s that can be unavailable during the update process.

In the **In-place Path**, the `Deployment` updates the template of the existing `ReplicaSet`, and the `ReplicaSet` controller updates the `Ship`s one by one to maintain availability.

### Recreate

All existing `Ship`s are killed before new ones are created.

- **Rotation Path**: Scales the old `ReplicaSet` to 0, waits for all `Ship`s to be deleted, then scales up the new `ReplicaSet`.
- **In-place Path**: Updates the template of the existing `ReplicaSet` and signals it to update all `Ship`s simultaneously (using the `tugboat.dev/update-strategy: all` annotation).

## Manifest Example

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web-server
  namespace: default
spec:
  replicas: 3
  selector:
    app: web
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 1
      maxUnavailable: 1
  shipTemplate:
    metadata:
      labels:
        app: web
    spec:
      shipClassName: medium
      image: "my-registry.local/web-server:v2"
```

## Status

The `DeploymentStatus` tracks:
- `replicas`: Total number of non-terminated `Ship`s targeted by this deployment.
- `updatedReplicas`: Total number of non-terminated `Ship`s targeted by this deployment that have the desired template spec.
- `readyReplicas`: Total number of ready `Ship`s targeted by this deployment.
