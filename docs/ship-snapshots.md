# Ship Snapshots

Ship snapshots capture the full state of a virtual machine, allowing you to back up and restore entire instances, including memory and disk state.

## `ShipSnapshot` Resource

A `ShipSnapshot` references a running or stopped `Ship`.

```yaml
apiVersion: snapshot.tugboat.cloud/v1
kind: ShipSnapshot
metadata:
  name: my-ship-snapshot
spec:
  shipName: my-ship
  includeVolumes: true
```

- `includeVolumes`: If `true`, Tugboat will also create `VolumeSnapshot`s for all CSI volumes attached to the Ship.

## Online vs. Offline Capture

- **Online**: Capture the state while the VM is running. This includes memory state.
- **Offline**: Capture the state while the VM is stopped. Only disk state is captured (if `includeVolumes` is true).

## Restoring a Ship

To restore a Ship from a snapshot, use the `restoreFromSnapshot` field in the `Ship` spec.

```yaml
spec:
  restoreFromSnapshot: my-ship-snapshot
```

Tugboat will ensure that the volumes are restored from the captured snapshots before starting the VM process.

## Runtime Support Matrix

| Runtime | Online | Offline | Notes |
| --- | --- | --- | --- |
| QEMU | yes | yes | Uses `savevm`/`loadvm`; not supported with VFIO passthrough. |
| Cloud Hypervisor | yes | yes | Uses `vm.snapshot` / `vm.restore`. Restore boots a fresh process. |

## Configuration

Runtimes require a `snapshot_dir` to be configured where snapshot metadata and memory state are stored.

Example `runtime/config.toml`:

```toml
snapshot_dir = "/var/lib/tugboat/snapshots"
```

## Known Limitations

- **Application Consistency**: Without a guest agent (e.g., QEMU Guest Agent), there is no guarantee of application-level consistency for online snapshots.
- **Cross-cluster Replication**: Ship snapshots are currently local to the cluster where they were created.
