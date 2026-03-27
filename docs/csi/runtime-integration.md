# Runtime Integration for CSI Volumes

Tugboat exposes CSI volumes to VM-based Ships using standard QEMU devices. The runtime integration handles the translation from a published CSI path on the host to a device or mount point within the guest VM.

## Volume Kinds

Tugboat distinguishes between two primary volume kinds for VMs:

### 1. Block Volumes (`VmVolumeKind::Block`)
If the CSI driver provides a block device (e.g., RBD, iSCSI), it is attached directly to the VM:
- **Host Side**: The CSI driver exposes a block device at a path like `/dev/sdb`.
- **QEMU Argument**: `-drive file=/dev/sdb,if=virtio,format=raw`
- **Guest Side**: Appears as a standard `virtio-blk` device (e.g., `/dev/vdb`).

### 2. Filesystem Volumes (`VmVolumeKind::Filesystem`)
If the CSI driver provides a mounted filesystem (e.g., NFS, CephFS, local path), it is shared with the VM via `virtio-9p`:
- **Host Side**: The CSI driver mounts the volume to a publish path.
- **QEMU Argument**: `-fsdev local,id=vol-id,path=/publish/path,security_model=none -device virtio-9p-pci,fsdev=vol-id,mount_tag=vol-tag`
- **Guest Side**: The guest can mount the share using the provided `mount_tag`. For example, `mount -t 9p vol-tag /mnt/storage`.

## Security Model

For `virtio-9p` shares, Tugboat uses `security_model=none`. This model is chosen because:
- **Guest Isolation**: The guest can manage its own permissions without being constrained by host-side user/group IDs.
- **Simplicity**: It avoids complex UID/GID mapping between the host and potentially many different guest OS configurations.
- **Isolation**: The publish path is already isolated within the Ship's mount namespace on the host, providing a layer of protection against unauthorized access.

## Virtiofs (Future Work)
Currently, Tugboat uses `virtio-9p` for filesystem sharing due to its broad compatibility. Support for `virtiofs`, which offers better performance and POSIX compliance, is planned for a future release.
