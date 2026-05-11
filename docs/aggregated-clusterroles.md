# Aggregated ClusterRoles

Aggregated ClusterRoles allow you to combine multiple `ClusterRole` resources into a single role. This feature is particularly useful for extending built-in roles (`admin`, `edit`, `view`) with permissions for new resources without modifying the original role definitions.

## How it works

A `ClusterRole` becomes "aggregated" when it includes an `aggregationRule`. This rule specifies one or more label selectors. The `tugboat-controller-manager` then finds all other `ClusterRole` resources that match these selectors and merges their rules into the aggregated role.

### Example

#### 1. Define the Aggregated Role (Parent)

```yaml
apiVersion: authorization/v1
kind: ClusterRole
metadata:
  name: monitor
aggregationRule:
  clusterRoleSelectors:
    - matchLabels:
        rbac.tugboat.cloud/aggregate-to-monitor: "true"
rules: [] # This will be automatically populated by the controller
```

#### 2. Define Extension Roles (Children)

```yaml
apiVersion: authorization/v1
kind: ClusterRole
metadata:
  name: prometheus-rules
  labels:
    rbac.tugboat.cloud/aggregate-to-monitor: "true"
rules:
  - apiGroups: [""]
    resources: ["ships", "nodes"]
    verbs: ["get", "list", "watch"]
```

The `monitor` role will now automatically include the rules from `prometheus-rules`.

## Built-in Aggregation

Tugboat's built-in roles (`admin`, `edit`, `view`) are also aggregated. You can extend them by creating a `ClusterRole` with the following labels:

- `rbac.tugboat.cloud/aggregate-to-admin: "true"`
- `rbac.tugboat.cloud/aggregate-to-edit: "true"`
- `rbac.tugboat.cloud/aggregate-to-view: "true"`

### Example: Adding custom resource permissions to `view`

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

## Implementation Details

- The aggregation is performed by the `AggregatedClusterRoleController` in `tugboat-controller-manager`.
- The controller periodically reconciles any `ClusterRole` with an `aggregationRule`.
- If you manually modify the `rules` field of an aggregated `ClusterRole`, the controller will overwrite your changes during the next reconciliation.
