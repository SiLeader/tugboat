# CSI Overview

Tugboat implements a full-featured Container Storage Interface (CSI) integration to support both Filesystem and Block storage for VM-based workloads (Ships).

## Architecture

The CSI implementation in Tugboat is distributed across several key components:

### 1. Tugboat Agent (`tugboat-agent`)
- **Node Service Client**: Executes `NodeStageVolume`, `NodePublishVolume`, `NodeUnpublishVolume`, and `NodeUnstageVolume` on the local CSI driver.
- **Volume Expansion**: Handles node-side filesystem expansion via `NodeExpandVolume` after a volume is resized.
- **Health & Stats**: Periodically queries `NodeGetVolumeStats` to report volume usage and health state back to the API server.
- **Mount Namespace Isolation**: Ensures that staging operations happen within the Ship's mount namespace to isolate the host from potentially malicious mount operations.

### 2. Tugboat Controller Manager (`tugboat-controller-manager`)
- **PvcProvisionerController**: Handles dynamic provisioning of `PersistentVolume` (PV) resources from `PersistentVolumeClaim` (PVC) requests using CSI `CreateVolume`.
- **PersistentVolumeCleanupController**: Finalizes PV deletion by calling CSI `DeleteVolume`.
- **Expansion Controller**: Initiates `ControllerExpandVolume` for CSI drivers that support online/offline expansion from the controller side.

### 3. CSI Operator (`tugboat-csi-operator`)
- **gRPC Wrapper**: Provides a Rust-friendly client for interacting with CSI driver unix-domain sockets.
- **Capability Negotiation**: Handles identity and capability discovery to ensure the driver supports the requested operations.

### 4. Tugboat Resources (`tugboat-resources`)
- **API Definition**: Defines the `PersistentVolume`, `PersistentVolumeClaim`, and `StorageClass` resources, including the `CsiPersistentVolumeSource` for driver-specific configuration.

## Key Concepts

- **Volume Modes**: Supports `Filesystem` (mounted via 9p in the guest) and `Block` (attached as a virtio-blk device).
- **Secrets**: Supports resolving secrets for `ControllerPublish`, `NodeStage`, `NodePublish`, and `NodeExpand` operations from first-class fields in the `PersistentVolume` spec.
- **Reconciliation**: Volume attachment/detachment is part of the Ship reconciliation loop. Currently, volume-related spec changes trigger a Ship recreation to ensure clean attachment/detachment.
