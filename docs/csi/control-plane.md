# Control Plane CSI Implementation

The Tugboat control plane manages the lifecycle of storage resources (`PersistentVolume`, `PersistentVolumeClaim`, `StorageClass`) and orchestrates dynamic provisioning and expansion.

## Components

### 1. PvcProvisionerController
Located in `tugboat-controller-manager/src/pvc_provisioner.rs`, this controller:
- **Dynamic Provisioning**: Watches for unbound PVCs and calls CSI `CreateVolume` on the appropriate driver (specified by the `StorageClass`).
- **PV Creation**: Upon successful provisioning, it creates a corresponding `PersistentVolume` (PV) resource and binds it back to the PVC.
- **Resize Handling**: Detects when a PVC's `requested_capacity_bytes` exceeds its current capacity and initiates `ControllerExpandVolume` on the CSI driver.

### 2. PersistentVolumeCleanupController
Located in `tugboat-controller-manager/src/pv_cleanup.rs`, this controller:
- **Deprovisioning**: Finalizes deletion of managed PVs by calling CSI `DeleteVolume`.
- **Resource Cleanup**: Ensures that orphaned CSI volumes are deleted from the storage backend.

## Storage Operations

### Dynamic Provisioning
When a user creates a `PersistentVolumeClaim` (PVC), the `PvcProvisionerController`:
1.  Identifies the `StorageClass` requested by the PVC.
2.  Resolves parameters and secrets from the `StorageClass`.
3.  Calls the CSI `CreateVolume` RPC with the requested capacity and parameters.
4.  Creates a `PersistentVolume` (PV) with the volume ID returned by the driver and the requested storage capacity.

### Volume Expansion
If a user increases the `requested_capacity_bytes` of a `PersistentVolumeClaim` (PVC):
1.  The `PvcProvisionerController` detects the increase.
2.  It calls the CSI `ControllerExpandVolume` RPC for the corresponding volume.
3.  Upon success, it updates the `PersistentVolume` (PV) status with the new capacity and sets `node_expansion_required = true` to signal the agent to perform node-side expansion.

## Secret Propagation

Secrets from `StorageClassSpec` are propagated to the provisioned PV:
- `controller_expand_secret_ref`
- `controller_publish_secret_ref`
- `node_expand_secret_ref`
- `node_publish_secret_ref`
- `node_stage_secret_ref`

These secrets are resolved and used by both the controller-manager and the agent as needed during volume lifecycle operations.
