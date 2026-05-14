# Topology-aware Scheduling

Tugboat supports topology-aware scheduling to optimize Ship placement based on data locality and resource availability. This is critical for stateful workloads where a Ship must be co-located with its volumes.

## Label Keys

The agent automatically populates topology labels on the `Node` resource. Well-known keys include:

- `topology.tugboat.cloud/region`: The geographic region (e.g., `us-east-1`).
- `topology.tugboat.cloud/zone`: The availability zone (e.g., `us-east-1a`).
- `topology.tugboat.cloud/host`: The specific physical host identifier.

These labels are derived from the agent configuration or discovered from the environment.

## `PV.spec.nodeAffinity`

When a `PersistentVolume` (PV) is dynamically provisioned, the CSI provisioner populates `spec.nodeAffinity` based on the `accessible_topology` reported by the CSI driver. This pins the PV to specific topology segments.

```yaml
apiVersion: v1
kind: PersistentVolume
metadata:
  name: pv-zone-a
spec:
  nodeAffinity:
    required:
      nodeSelectorTerms:
        - matchExpressions:
            - key: topology.tugboat.cloud/zone
              operator: In
              values:
                - zone-a
```

## `StorageClass.volumeBindingMode`

To coordinate Ship scheduling with volume provisioning, use `volumeBindingMode: WaitForFirstConsumer` in the `StorageClass`.

- `Immediate`: PV is provisioned as soon as the PVC is created. This may lead to the PV being in a zone where the Ship cannot be scheduled.
- `WaitForFirstConsumer`: PV provisioning is delayed until a Ship using the PVC is scheduled. The scheduler ensures the Ship lands on a node that can access the required topology.

The scheduler uses the `volume.tugboat.cloud/selected-node` annotation on the PVC to signal to the provisioner which node was chosen.

## `Ship.spec.affinity`

Ships can express placement preferences or requirements using `affinity`.

### `nodeAffinity`

Pins a Ship to specific nodes based on labels.

```yaml
spec:
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

### `shipAffinity` and `shipAntiAffinity`

Allows co-locating or spreading Ships based on labels of other Ships already running on nodes.

```yaml
spec:
  affinity:
    shipAntiAffinity:
      requiredDuringSchedulingIgnoredDuringExecution:
        - labelSelector:
            matchExpressions:
              - key: app
                operator: In
                values:
                  - my-app
          topologyKey: topology.tugboat.cloud/zone
```

## `topologySpreadConstraints`

Ensures Ships are spread evenly across topology domains (e.g., zones).

```yaml
spec:
  topologySpreadConstraints:
    - maxSkew: 1
      topologyKey: topology.tugboat.cloud/zone
      whenUnsatisfiable: DoNotSchedule
      labelSelector:
        matchLabels:
          app: my-app
```

## Scheduler Plugin Reference

The following plugins are used for topology-aware scheduling:

- `VolumeTopology`: Ensures the node satisfies the `PV.spec.nodeAffinity`.
- `NodeAffinity`: Enforces `Ship.spec.affinity.nodeAffinity`.
- `ShipAffinity`: Enforces `Ship.spec.affinity.shipAffinity`.
- `ShipAntiAffinity`: Enforces `Ship.spec.affinity.shipAntiAffinity`.
- `TopologySpread`: Enforces `Ship.spec.topologySpreadConstraints`.
- `ImageLocality`: Scores nodes based on whether the required VM image is already present.

These can be enabled or disabled via the scheduler configuration file.
