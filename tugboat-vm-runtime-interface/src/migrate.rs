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

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmMigrateRequest {
    pub id: String,
    pub destination_address: String,
    pub destination_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bandwidth_bytes_per_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downtime_limit_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xbzrle_cache_size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub postcopy_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmMigrateCancelRequest {
    pub id: String,
}

/// Optional QEMU migration tuning parameters sourced from the ShipClass
/// `spec.migration` field. All fields are optional; absent values fall back to
/// the runtime's built-in defaults. `postcopy_enabled` should only be enabled
/// on reliable low-latency networks because page transfer failures during
/// post-copy can crash the guest.
#[derive(Debug, Clone, Default)]
pub struct VmMigrationParams {
    pub max_bandwidth_bytes_per_sec: Option<u64>,
    pub downtime_limit_ms: Option<u64>,
    pub xbzrle_cache_size_bytes: Option<u64>,
    pub postcopy_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VmMigrationPhase {
    None,
    Setup,
    Active,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmMigrationStatusResponse {
    pub phase: VmMigrationPhase,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_transferred: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_remaining: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_dirty_rate_mbps: Option<f64>,
}
