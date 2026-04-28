# ReplicaSet

A `ReplicaSet` ensures that a specified number of `Ship` (VM) replicas are running at any given time. It is often used
to guarantee the availability of a specific number of identical Ships.

## How it works

A `ReplicaSet` is defined with a selector that identifies which `Ship`s it manages, a number of replicas it should
maintain, and a `Ship` template specifying the data for new `Ship`s it should create to meet the number of replicas.

The `ReplicaSet` controller fulfills this by creating or deleting `Ship`s as needed. When a `ReplicaSet` needs to create
new `Ship`s, it uses its `Ship` template.

## Manifest Example

```yaml
apiVersion: apps/v1
kind: ReplicaSet
metadata:
  name: sample-rs
  namespace: default
spec:
  replicas: 3
  selector:
    app: web
  shipTemplate:
    metadata:
      labels:
        app: web
    spec:
      shipClass: small
      image: "my-registry.local/web-app:v1"
```

## Key Fields

- `spec.replicas`: The desired number of replicas. Defaults to 1.
- `spec.selector`: A label selector used to identify the `Ship`s that belong to this `ReplicaSet`.
- `spec.shipTemplate`: The template used to create new `Ship`s. It includes `metadata` (labels and annotations) and
  `spec` (the VM configuration).

## Update Strategy

While `ReplicaSet`s are primarily managed by `Deployment`s for rollouts, they can be updated directly.

Tugboat supports an in-place update for `ReplicaSet`s when the `tugboat.cloud/update-strategy: all` annotation is
present. In this mode, if the `shipTemplate` is updated, the `ReplicaSet` controller will update all existing `Ship`s
managed by it to match the new template simultaneously. If this annotation is not present, the `ReplicaSet` controller
typically performs updates in a way that minimizes downtime (e.g., one by one).
