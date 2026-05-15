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

use crate::cmd::qmp::{connect_qmp, execute_with_timeout};
use crate::execute::vm::QemuVmConfig;
use clap::Parser;
use qapi::qmp::human_monitor_command;
use tugboat_runtime_common::config::load_config;
use tugboat_runtime_common::snapshot::ship_snapshot_dir;
use tugboat_runtime_common::validate::validate_safe_id;
use tugboat_vm_runtime_interface::snapshot::{
    VmSnapshotCreateRequest, VmSnapshotCreateResponse, VmSnapshotDeleteRequest, VmSnapshotEntry,
    VmSnapshotListRequest, VmSnapshotListResponse, VmSnapshotMode, VmSnapshotRestoreRequest,
};

const RUNTIME_NAME: &str = "qemu";

#[derive(Debug, Parser)]
pub struct SnapshotCreateArgs {
    #[arg(help = "Path to the snapshot create request config file or - for stdin")]
    config: String,
}

#[derive(Debug, Parser)]
pub struct SnapshotDeleteArgs {
    #[arg(help = "Path to the snapshot delete request config file or - for stdin")]
    config: String,
}

#[derive(Debug, Parser)]
pub struct SnapshotRestoreArgs {
    #[arg(help = "Path to the snapshot restore request config file or - for stdin")]
    config: String,
}

#[derive(Debug, Parser)]
pub struct SnapshotListArgs {
    #[arg(help = "Path to the snapshot list request config file or - for stdin")]
    config: String,
}

pub async fn snapshot_create(config: QemuVmConfig, args: SnapshotCreateArgs) -> crate::Result<()> {
    let req: VmSnapshotCreateRequest = load_config(args.config)?;
    req.validate()
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    validate_safe_id(&req.ship_id, "vm id")?;

    let handle = derive_snapshot_handle(&req.ship_id);
    // Ensure the per-Ship staging directory exists, even though internal
    // savevm snapshots are stored inside the qcow2 today. This reserves
    // the on-disk location for external snapshots and metadata.
    let staging_dir = ship_snapshot_dir(&config.snapshot_dir_path(), &req.ship_id)
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    std::fs::create_dir_all(&staging_dir)?;
    let hmp = format!("savevm {handle}");
    run_hmp(&config, &req.ship_id, &hmp, "Timed out issuing savevm").await?;

    let response = VmSnapshotCreateResponse {
        handle,
        runtime: RUNTIME_NAME.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        size_bytes: None,
    };
    write_json(&response)?;

    // The mode is currently advisory — savevm pauses briefly even in
    // online mode, and rejects the call when the guest is not in a
    // suitable state, mirroring QEMU's built-in semantics.
    let _ = VmSnapshotMode::default();
    Ok(())
}

pub async fn snapshot_delete(config: QemuVmConfig, args: SnapshotDeleteArgs) -> crate::Result<()> {
    let req: VmSnapshotDeleteRequest = load_config(args.config)?;
    req.validate()
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    validate_safe_id(&req.ship_id, "vm id")?;

    let hmp = format!("delvm {}", req.handle);
    run_hmp(&config, &req.ship_id, &hmp, "Timed out issuing delvm").await?;
    Ok(())
}

pub async fn snapshot_restore(
    config: QemuVmConfig,
    args: SnapshotRestoreArgs,
) -> crate::Result<()> {
    let req: VmSnapshotRestoreRequest = load_config(args.config)?;
    req.validate()
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    validate_safe_id(&req.ship_id, "vm id")?;

    let hmp = format!("loadvm {}", req.handle);
    run_hmp(&config, &req.ship_id, &hmp, "Timed out issuing loadvm").await?;
    Ok(())
}

pub async fn snapshot_list(config: QemuVmConfig, args: SnapshotListArgs) -> crate::Result<()> {
    let req: VmSnapshotListRequest = load_config(args.config)?;
    req.validate()
        .map_err(|e| crate::Error::Validation(e.to_string()))?;
    validate_safe_id(&req.ship_id, "vm id")?;

    let output = run_hmp(
        &config,
        &req.ship_id,
        "info snapshots",
        "Timed out issuing info snapshots",
    )
    .await?;
    let snapshots = parse_info_snapshots(&output);
    let response = VmSnapshotListResponse { snapshots };
    write_json(&response)?;
    Ok(())
}

async fn run_hmp(
    config: &QemuVmConfig,
    ship_id: &str,
    command_line: &str,
    timeout_message: &str,
) -> crate::Result<String> {
    let (qmp, _handle) = connect_qmp(config.get_uds_path(ship_id))
        .await?
        .spawn_tokio();
    let output = execute_with_timeout(
        qmp.execute(human_monitor_command {
            command_line: command_line.to_string(),
            cpu_index: None,
        }),
        timeout_message,
    )
    .await?;
    let trimmed = output.trim();
    if trimmed.to_ascii_lowercase().starts_with("error") {
        return Err(crate::Error::ActionFailed(trimmed.to_string()));
    }
    Ok(output)
}

fn write_json<T: serde::Serialize>(value: &T) -> crate::Result<()> {
    serde_json::to_writer(std::io::stdout(), value).map_err(crate::Error::Json)
}

fn derive_snapshot_handle(ship_id: &str) -> String {
    // savevm tags are bounded to 256 chars and need to be filesystem-safe.
    // Combine the ship id with a UTC timestamp and a short random suffix.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let suffix = &suffix[..8];
    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    format!("{ship_id}-{timestamp}-{suffix}")
}

/// Parse the textual output of `info snapshots` into snapshot entries.
/// QEMU's HMP output looks roughly like:
///
/// ```text
/// List of snapshots present on all disks:
/// ID        TAG               VM SIZE                DATE       VM CLOCK     ICOUNT
/// --        snap-1            210M 2026-05-14 12:00:00 00:00:00.000
/// ```
///
/// The exact column widths vary between QEMU versions, so we tolerate
/// any whitespace separation and skip header / separator rows.
pub(crate) fn parse_info_snapshots(text: &str) -> Vec<VmSnapshotEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("List of snapshots")
            || line.starts_with("There is no snapshot")
            || (line.starts_with("ID") && line.contains("TAG"))
        {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        let tag = fields[1].to_string();
        let size_field = fields[2];
        let size_bytes = parse_size_to_bytes(size_field);
        let date = if fields.len() >= 5 {
            format!("{} {}", fields[3], fields[4])
        } else {
            String::new()
        };
        entries.push(VmSnapshotEntry {
            handle: tag,
            created_at: date,
            size_bytes,
        });
    }
    entries
}

fn parse_size_to_bytes(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let (number, suffix) = value.split_at(
        value
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(value.len()),
    );
    let n: f64 = number.parse().ok()?;
    let multiplier = match suffix.to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "K" | "KB" | "KIB" => 1024.0,
        "M" | "MB" | "MIB" => 1024.0 * 1024.0,
        "G" | "GB" | "GIB" => 1024.0 * 1024.0 * 1024.0,
        "T" | "TB" | "TIB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * multiplier) as u64)
}

#[cfg(test)]
mod tests {
    use super::{derive_snapshot_handle, parse_info_snapshots, parse_size_to_bytes};

    #[test]
    fn derive_snapshot_handle_starts_with_ship_id() {
        let handle = derive_snapshot_handle("ship-a");
        assert!(handle.starts_with("ship-a-"));
    }

    #[test]
    fn parse_info_snapshots_skips_header_and_returns_rows() {
        let text = "List of snapshots present on all disks:\nID        TAG               VM SIZE                DATE       VM CLOCK     ICOUNT\n--        snap-1            210M 2026-05-14 12:00:00 00:00:00.000\n";
        let entries = parse_info_snapshots(text);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].handle, "snap-1");
        assert_eq!(entries[0].size_bytes, Some(210 * 1024 * 1024));
        assert_eq!(entries[0].created_at, "2026-05-14 12:00:00");
    }

    #[test]
    fn parse_info_snapshots_handles_empty_output() {
        let entries = parse_info_snapshots("There is no snapshot available.\n");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_size_to_bytes_handles_units() {
        assert_eq!(parse_size_to_bytes("210M"), Some(210 * 1024 * 1024));
        assert_eq!(parse_size_to_bytes("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size_to_bytes("512"), Some(512));
        assert_eq!(parse_size_to_bytes("garbage"), None);
    }
}
