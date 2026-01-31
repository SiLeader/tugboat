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

use nix::libc::umask;
use nix::mount::{MsFlags, mount};
use nix::sched::{CloneFlags, unshare};
use nix::sys::signal::{SigHandler, Signal, signal};
use nix::unistd::{Gid, Uid, chdir, fork, setgid, setsid, setuid};
use std::path::{Path, PathBuf};
use std::process::exit;
use tracing::{debug, info};
use tugboat_vm_runtime_interface::run::VmExecUser;

pub(crate) fn create_and_enter_to_network_namespace(ship_id: &str) -> Result<(), crate::Error> {
    debug!(
        "Create and enter to network namespace. pid = {}",
        std::process::id()
    );
    unshare(CloneFlags::CLONE_NEWNET)?;
    info!(
        "Successfully created network namespace. pid = {}",
        std::process::id()
    );

    let netns = format!("/var/run/netns/{ship_id}");
    debug!("Creating bind mount netns path '{netns}'");
    std::fs::create_dir_all("/var/run/netns")?;
    std::fs::File::create(&netns)?;
    debug!(
        "/proc/self/ns/net exists: {}",
        PathBuf::from("/proc/self/ns/net").exists()
    );
    mount(
        Some("/proc/self/ns/net"),
        netns.as_str(),
        Option::<&Path>::None,
        MsFlags::MS_BIND,
        Option::<&Path>::None,
    )?;

    info!("Successfully created bind mount netns path '{netns}'");

    Ok(())
}

pub(crate) fn change_running_user_and_group(user: &VmExecUser) -> Result<(), crate::Error> {
    debug!("Change running user and group");
    if let Some(user) = user.user {
        info!("Change running user to {}", user);
        setuid(Uid::from_raw(user))?;
    }
    if let Some(group) = user.group {
        info!("Change running group to {}", group);
        setgid(Gid::from_raw(group))?;
    }

    Ok(())
}

pub(crate) fn daemonize() {
    debug!("Daemonize start");
    let pid = unsafe { fork() }.expect("Failed to fork");
    if pid.is_parent() {
        exit(0);
    }

    setsid().expect("Failed to setsid");

    unsafe { signal(Signal::SIGHUP, SigHandler::SigIgn) }.expect("Failed to disable SIGHUP");
    unsafe { signal(Signal::SIGCHLD, SigHandler::SigIgn) }.expect("Failed to disable SIGCHLD");

    let pid = unsafe { fork() }.expect("Failed to fork");
    if pid.is_parent() {
        exit(0);
    }
    unsafe { umask(0) };
    chdir("/").expect("Failed to chdir");
    info!("Successfully daemonized");
}
