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

use crate::execute::vm::QemuVmConfig;
use tokio::process::Command;
use tracing::{error, info};

#[allow(dead_code)]
pub async fn setup_tap_redirect(config: &QemuVmConfig, bridge: &str) -> Result<(), crate::Error> {
    info!("Starting TAP device setup and TC redirect...");
    let ip: &str = &config.executables.ip;
    let tc: &str = &config.executables.tc;

    let tap = bridge.replace("eth", "tap");

    // Create tap device
    run_command(ip, &["tuntap", "add", "dev", &tap, "mode", "tap"]).await?;

    // Link up
    run_command(ip, &["link", "set", bridge, "up"]).await?;
    run_command(ip, &["link", "set", &tap, "up"]).await?;

    // Packet transfer between bridge and tap

    // eth0 -> tap0 (Ingress)
    run_command(tc, &["qdisc", "add", "dev", bridge, "ingress"]).await?;
    // redirect all packets to tap
    run_command(
        tc,
        &[
            "filter", "add", "dev", bridge, "parent", "ffff:", "protocol", "all", "u32", "match",
            "u32", "0", "0", "action", "mirred", "egress", "redirect", "dev", &tap,
        ],
    )
    .await?;

    // tap0 -> eth0 (Egress)
    run_command(tc, &["qdisc", "add", "dev", &tap, "ingress"]).await?;
    // redirect all packets to bridge
    run_command(
        tc,
        &[
            "filter", "add", "dev", &tap, "parent", "ffff:", "protocol", "all", "u32", "match",
            "u32", "0", "0", "action", "mirred", "egress", "redirect", "dev", bridge,
        ],
    )
    .await?;

    info!("Network setup completed successfully.");
    Ok(())
}

async fn run_command(program: &str, args: &[&str]) -> Result<(), crate::Error> {
    let output = Command::new(program).args(args).output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Command failed: {} {:?}: {}", program, args, stderr);
        return Err(crate::Error::NetworkSetupFailed(format!(
            "Command failed: {} {:?}: {}",
            program, args, stderr
        )));
    }
    Ok(())
}
