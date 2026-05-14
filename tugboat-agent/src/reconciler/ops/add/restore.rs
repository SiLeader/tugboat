// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Helpers for the `spec.restoreFromSnapshot` branch of the add path.
//!
//! When a Ship is created with `spec.restoreFromSnapshot = "<snap>"`, the
//! agent must:
//!
//! 1. Look up the named `ShipSnapshot` in the same namespace and verify
//!    it is `Ready`.
//! 2. Capture its runtime-specific `status.handle` so the create path
//!    can issue a runtime restore instead of a fresh `add`.
//!
//! Step 2's hookup into [`RuntimeOperator::create`] is intentionally
//! left as a follow-up — the field is plumbed end-to-end but the
//! actual VM boot still goes through the normal path. The agent emits
//! a Ship condition so operators can see the snapshot was resolved.

#![allow(dead_code)]

use crate::reconciler::error::ReconcileError;
use tugboat_resources::manifests::core::v1::{Ship, ShipSnapshot};

/// Validated restore intent: pairs the runtime-specific snapshot handle
/// with the runtime name that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSnapshotRestore {
    pub(crate) snapshot_name: String,
    pub(crate) handle: String,
    pub(crate) runtime: String,
}

/// Returns `Some(name)` when this Ship requests a restore, `None`
/// otherwise.
pub(crate) fn requested_restore_snapshot(ship: &Ship) -> Option<&str> {
    ship.spec
        .as_ref()
        .and_then(|spec| spec.restore_from_snapshot.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Validates the lookup result of the `ShipSnapshot` referenced by
/// `restoreFromSnapshot`, returning a resolved restore intent that can
/// drive a runtime restore.
pub(crate) fn resolve_restore_snapshot(
    snapshot_name: &str,
    snapshot: Option<&ShipSnapshot>,
) -> Result<ResolvedSnapshotRestore, ReconcileError> {
    let Some(snapshot) = snapshot else {
        return Err(ReconcileError::Validation(format!(
            "spec.restoreFromSnapshot references missing ShipSnapshot '{snapshot_name}'"
        )));
    };
    let status = snapshot.status.as_ref().ok_or_else(|| {
        ReconcileError::Validation(format!(
            "ShipSnapshot '{snapshot_name}' has no status yet; it is not ready for restore"
        ))
    })?;
    if status.phase != "Ready" {
        return Err(ReconcileError::Validation(format!(
            "ShipSnapshot '{snapshot_name}' is not Ready (phase = '{}')",
            status.phase
        )));
    }
    let handle = status
        .handle
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ReconcileError::Validation(format!(
                "ShipSnapshot '{snapshot_name}' is Ready but has no status.handle yet"
            ))
        })?;
    let runtime = status
        .runtime
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ReconcileError::Validation(format!(
                "ShipSnapshot '{snapshot_name}' is Ready but has no status.runtime yet"
            ))
        })?;
    Ok(ResolvedSnapshotRestore {
        snapshot_name: snapshot_name.to_string(),
        handle: handle.to_string(),
        runtime: runtime.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tugboat_resources::manifests::core::v1::{ShipSnapshotStatus, ShipSpec};
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    fn ship_with_restore(name: Option<&str>) -> Ship {
        Ship {
            spec: Some(ShipSpec {
                restore_from_snapshot: name.map(str::to_string),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn ready_snapshot(handle: &str, runtime: &str) -> ShipSnapshot {
        ShipSnapshot {
            object_meta: Some(ObjectMeta {
                name: Some("snap-a".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            }),
            status: Some(ShipSnapshotStatus {
                phase: "Ready".to_string(),
                handle: Some(handle.to_string()),
                runtime: Some(runtime.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn requested_restore_snapshot_returns_some_when_set() {
        assert_eq!(
            requested_restore_snapshot(&ship_with_restore(Some("snap-a"))),
            Some("snap-a")
        );
    }

    #[test]
    fn requested_restore_snapshot_treats_whitespace_as_unset() {
        assert!(requested_restore_snapshot(&ship_with_restore(Some("   "))).is_none());
        assert!(requested_restore_snapshot(&ship_with_restore(None)).is_none());
    }

    #[test]
    fn resolve_restore_snapshot_returns_handle_when_ready() {
        let snap = ready_snapshot("snap-handle", "qemu");
        let resolved = resolve_restore_snapshot("snap-a", Some(&snap)).unwrap();
        assert_eq!(resolved.snapshot_name, "snap-a");
        assert_eq!(resolved.handle, "snap-handle");
        assert_eq!(resolved.runtime, "qemu");
    }

    #[test]
    fn resolve_restore_snapshot_rejects_missing() {
        let err = resolve_restore_snapshot("snap-a", None).unwrap_err();
        assert!(matches!(err, ReconcileError::Validation(message) if message.contains("missing")));
    }

    #[test]
    fn resolve_restore_snapshot_rejects_not_ready() {
        let mut snap = ready_snapshot("snap-handle", "qemu");
        snap.status.as_mut().unwrap().phase = "Capturing".to_string();
        let err = resolve_restore_snapshot("snap-a", Some(&snap)).unwrap_err();
        assert!(
            matches!(err, ReconcileError::Validation(message) if message.contains("not Ready"))
        );
    }

    #[test]
    fn resolve_restore_snapshot_rejects_missing_handle() {
        let mut snap = ready_snapshot("snap-handle", "qemu");
        snap.status.as_mut().unwrap().handle = None;
        let err = resolve_restore_snapshot("snap-a", Some(&snap)).unwrap_err();
        assert!(
            matches!(err, ReconcileError::Validation(message) if message.contains("status.handle"))
        );
    }
}
