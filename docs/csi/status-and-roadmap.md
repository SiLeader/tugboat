# Status and Roadmap for CSI

Tugboat's CSI implementation is under active development. This document outlines the current status, known limitations, and planned features.

## Current Status

- [x] **Dynamic Provisioning**: `PersistentVolume` creation from `PersistentVolumeClaim`.
- [x] **Provisioner Secrets**: `StorageClassSpec` supports `controller_create_secret_ref`.
- [x] **Mount Options**: `StorageClassSpec` supports `mount_options`, propagated to `NodeStage`/`NodePublish`.
- [x] **Volume Lifecycle**: Full `NodeStage` → `NodePublish` → `NodeUnpublish` → `NodeUnstage` pipeline.
- [x] **Volume Expansion**: Support for `ControllerExpandVolume` and `NodeExpandVolume` (both during Ship creation and live for running Ships).
- [x] **Mount Isolation**: Execution of CSI operations within Ship-specific mount namespaces.
- [x] **VM Runtime**: Support for `virtio-blk` (Block) and `virtio-9p` (Filesystem) devices.
- [x] **Monitoring**: Volume usage and health monitoring via `NodeGetVolumeStats`.

## Known Limitations

- **Topology Support**: Topology-aware scheduling and late binding are not yet supported.
- **Live Attach/Detach**: Adding or removing volumes in a Ship's spec requires a full Ship recreate.

## Roadmap

### Phase 1: Feature Completeness
- [x] Add `mount_options` support.
- [x] Add `controller_create_secret_ref` for provisioning-time secrets.
- [x] Support live volume expansion in `reconciler/ops/modify.rs` without requiring Ship recreate.
- [ ] Improve agent recovery for missing or corrupted volume state files.

### Phase 2: Advanced Scheduling
- [ ] Implement CSI topology support (`AllowedTopologies`, `NodeAffinity`).
- [ ] Support `VolumeBindingMode: WaitForFirstConsumer` for late-binding volumes.
- [ ] Add scheduler plugins for CSI attach limits and topology constraints.

### Phase 3: Enhanced Functionality
- [ ] Support volume snapshots (`VolumeSnapshot`, `VolumeSnapshotContent`).
- [ ] Support volume cloning and restore from snapshots.
- [ ] Explore `virtiofs` as a higher-performance alternative to `virtio-9p`.
- [ ] Support in-place volume attach/detach where supported by the driver.
