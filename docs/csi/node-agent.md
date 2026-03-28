# Node Agent CSI Implementation

The `tugboat-agent` is responsible for attaching and detaching volumes on the local node. It implements the standard CSI Node service operations with a focus on isolation and reliability.

## Volume Life Cycle (Ship Reconciliation)

When a Ship is created or updated, the agent's reconciler (`tugboat-agent/src/reconciler/volume.rs`) resolves the required volumes:

1.  **Secret Resolution**: The agent resolves any required CSI secrets (e.g., `controller_publish_secret_ref`, `node_stage_secret_ref`) from the API server.
2.  **Controller Publish**: If the driver requires `ControllerPublishVolume` and the volume is not yet published to the node, the agent executes it (typically via a controller-manager, but the agent manages the state).
3.  **Node Stage**: For drivers that support `NodeStageVolume`, the agent calls this operation to mount the volume into a global staging path (e.g., `/var/lib/tugboat/csi/staging/`).
4.  **Node Publish**: The agent calls `NodePublishVolume` to bind-mount the volume from the staging path to a Ship-specific publish path (e.g., `/var/lib/tugboat/ships/<ship-id>/volumes/<volume-name>/`).
5.  **Runtime Integration**: The publish path is then passed to the VM runtime to be exposed to the guest VM.

## Mount Namespace Isolation

To improve security, Tugboat ensures that CSI mount operations happen within the Ship's mount namespace (`tugboat-agent/src/mountns.rs`).

- **Namespace Creation**: Before `NodeStageVolume` or `NodePublishVolume` is called, a dedicated mount namespace is created for the Ship.
- **In-Namespace Execution**: The CSI driver's mount commands are executed within this namespace, preventing potentially compromised drivers from affecting the host's mount table.
- **Persistence**: The mount namespace is bound to the host filesystem to ensure it persists across agent restarts until the Ship is deleted.

## Volume Modes

- **Block**: The CSI driver provides a block device. The agent passes the device path to the VM runtime, which attaches it as a `virtio-blk` device.
- **Filesystem**: The CSI driver mounts a filesystem. The agent passes the mount point to the VM runtime, which shares it with the guest via `virtio-9p`.

## Expansion and Health Monitoring

- **Node Expansion**: After a volume is resized on the control plane, the agent detects the capacity change and calls `NodeExpandVolume` if the driver supports it (`NodeCapability::ExpandVolume`).
- **Health Checks**: The agent uses `NodeGetVolumeStats` to monitor the volume's health and usage. This data is surfaced in the `PersistentVolumeStatus` and `PersistentVolumeClaimStatus` resources.

## Recovery

The agent persists volume state in JSON files under the publish directory. Upon restart, the agent reads these files to recover the state of already-running Ships and their attached volumes without re-querying the API server or the CSI driver immediately.
