# Volume Snapshots and Clones

Volume snapshots provide a way to create point-in-time copies of your persistent volumes. These snapshots can be used for backup, data distribution, or to create new volumes (clones).

## Resource Overview

- `VolumeSnapshotClass`: Defines the CSI driver and parameters for creating snapshots.
- `VolumeSnapshot`: A user's request for a snapshot of a specific PVC.
- `VolumeSnapshotContent`: The actual snapshot resource in the storage system (cluster-scoped).

## Lifecycle

1. **Create**: User creates a `VolumeSnapshot` referencing a `PersistentVolumeClaim`.
2. **Bind**: The `VolumeSnapshotController` creates a `VolumeSnapshotContent` (or binds to an existing one) and links them.
3. **Ready**: The CSI driver performs the snapshot operation. Once successful, `status.readyToUse` becomes `true`.
4. **Delete**: When a `VolumeSnapshot` is deleted, the `VolumeSnapshotContent` is also deleted (if the `deletionPolicy` is `Delete`).

## Restore and Clone

You can create a new PVC from a snapshot or an existing PVC by specifying the `dataSource` field.

### Restore from Snapshot

```yaml
spec:
  dataSource:
    name: my-snapshot
    kind: VolumeSnapshot
    apiGroup: snapshot
```

### Clone from PVC

```yaml
spec:
  dataSource:
    name: source-pvc
    kind: PersistentVolumeClaim
```

Note: Clones and restores are subject to the capabilities of the underlying CSI driver.

## Combination with WFFC

When using `volumeBindingMode: WaitForFirstConsumer` (WFFC), the scheduler ensures that the new volume is provisioned on a node that has access to the snapshot or source volume.

## Troubleshooting

- **Missing driver capability**: Ensure the CSI driver supports `CREATE_DELETE_SNAPSHOT` and `LIST_SNAPSHOTS`.
- **Snapshot stuck `Pending`**: Check the `tugboat-controller-manager` logs for errors from the CSI driver.
- **Content stuck without handle**: This usually indicates a failure in the CSI `CreateSnapshot` call. Verify the source volume exists and is accessible.
