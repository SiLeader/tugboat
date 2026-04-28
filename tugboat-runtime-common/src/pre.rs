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

use nix::libc::{dup2, umask};
use nix::mount::{MsFlags, mount};
use nix::sched::{CloneFlags, setns, unshare};
use nix::sys::signal::{SigHandler, Signal, signal};
use nix::unistd::{Gid, Uid, chdir, fork, setgid, setsid, setuid};
use std::fs::File;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::exit;
use tracing::{debug, info};
use tugboat_vm_runtime_interface::run::VmExecUser;
use tugboat_vm_runtime_interface::run::mount_namespace_path;

use crate::validate::validate_safe_id;

pub fn enter_mount_namespace(ship_id: &str) -> Result<(), crate::Error> {
    validate_safe_id(ship_id, "ship_id")?;
    let mountns = mount_namespace_path(ship_id);
    if !PathBuf::from(&mountns).exists() {
        debug!("No mount namespace found for ship '{ship_id}', skipping");
        return Ok(());
    }

    debug!("Entering mount namespace '{mountns}'");
    let namespace = File::open(&mountns)?;
    setns(&namespace, CloneFlags::CLONE_NEWNS)?;
    info!("Successfully entered mount namespace '{mountns}'");
    Ok(())
}

pub fn create_and_enter_to_network_namespace(ship_id: &str) -> Result<(), crate::Error> {
    validate_safe_id(ship_id, "ship_id")?;
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
    // Use O_NOFOLLOW to prevent symlink attacks between file creation and bind mount.
    File::options()
        .write(true)
        .create_new(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&netns)?;
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

pub fn change_running_user_and_group(user: &VmExecUser) -> Result<(), crate::Error> {
    debug!("Change running user and group");
    if let Some(group) = user.group {
        info!("Change running group to {}", group);
        setgid(Gid::from_raw(group))?;
    }
    if let Some(user) = user.user {
        info!("Change running user to {}", user);
        setuid(Uid::from_raw(user))?;
    }

    Ok(())
}

pub fn daemonize() -> Result<(), crate::Error> {
    debug!("Daemonize start");
    let pid = unsafe { fork() }?;
    if pid.is_parent() {
        exit(0);
    }

    setsid()?;

    unsafe { signal(Signal::SIGHUP, SigHandler::SigIgn) }?;
    unsafe { signal(Signal::SIGCHLD, SigHandler::SigIgn) }?;

    let pid = unsafe { fork() }?;
    if pid.is_parent() {
        exit(0);
    }
    unsafe { umask(0) };
    chdir("/")?;
    let devnull = File::open("/dev/null")?;
    let null_fd = devnull.as_raw_fd();
    unsafe {
        if dup2(null_fd, 0) == -1 || dup2(null_fd, 1) == -1 || dup2(null_fd, 2) == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    info!("Successfully daemonized");
    Ok(())
}
