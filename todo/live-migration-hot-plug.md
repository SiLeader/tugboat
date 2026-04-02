Live Migration and Dynamic Resource Adjustment
============================================

Goal
Implement live migration for Ships between nodes to allow zero-downtime rescheduling, and support dynamic resource adjustment (CPU/memory hotplug) for running VMs without recreation.

Current evidence
- `tugboat-agent` currently filters Watch events strictly by `spec.nodeName`, ignoring target nodes during a transition.
- `tugboat-runtime` starts QEMU without incoming migration options and lacks configuration for hotpluggable resources (e.g., `maxcpus`, `maxmem`, `slots`).
- `tugboat-vm-runtime-interface` and `tugboat-runtime` do not expose QMP commands for migration (`migrate`, `query-migrate`) or hotplugging (`device_add`, `object-add`).
- `tugboat-agent/src/reconciler/ops/modify.rs` triggers a full `reconcile_recreate` whenever the `ship_class` (CPU/Memory) changes.

What to implement
- [API Extension] Extend `ShipSpec` with `target_node_name` to indicate the destination node for migration. Add a `migration` field of type `ShipMigrationStatus` to `ShipStatus` to track the state (e.g., Pending, Preparing, Ready, Migrating, Completed, Failed).
- [QEMU Configuration] Update `tugboat-runtime/src/execute/vm/qemu/mod.rs` to support `-incoming tcp:[::]:<port>` mode for the target node. Also, append `maxcpus` to the `-smp` argument and `maxmem`/`slots` to the `-m` argument to enable hotplugging.
- [Live Migration Flow]
  1. Modify `tugboat-agent` to reconcile Ships where `target_node_name` matches the local node.
  2. On the target node, prepare CSI/CNI, start QEMU in incoming mode, and update the migration status to `Ready`.
  3. On the source node, detect the `Ready` status, and invoke the QMP `migrate` command via `tugboat-runtime` to start the transfer.
  4. Poll `query-migrate` until completion. Once finished, the source node cleans up its VM instance, updates `spec.node_name` to the target, clears `target_node_name`, and sets the status to `Completed`.
- [Hotplug Flow]
  1. Define new interfaces in `tugboat-vm-runtime-interface` for resource updates (e.g., `VmUpdateResourcesRequest`).
  2. Implement QMP commands in `tugboat-runtime` to add/remove vCPUs and memory (via `pc-dimm` devices).
  3. Update `reconcile_modified` in `tugboat-agent` with a helper (e.g., `is_only_cpu_memory_changed`) to determine if a spec change is limited to resources.
  4. If true, bypass `reconcile_recreate` and invoke the hotplug commands via the `RuntimeOperator` to apply changes dynamically.

Implementation surfaces
- tugboat-resources/proto/core/v1/ship.proto
- tugboat-vm-runtime-interface/src/
- tugboat-runtime/src/
- tugboat-agent/src/reconciler/mod.rs
- tugboat-agent/src/reconciler/ops/modify.rs

Acceptance criteria
- Setting `target_node_name` on a running `Ship` successfully migrates the VM to the target node without stopping the workload.
- Upon successful migration, `node_name` reflects the new node, `target_node_name` is cleared, and the source VM is safely terminated.
- Changing the `shipClass` (CPU or memory) of a running `Ship` dynamically updates the VM resources via hotplug without recreating the VM.
- Errors during migration or hotplugging are safely caught, reported in the Ship's status conditions, and leave the VM in a recoverable state.
