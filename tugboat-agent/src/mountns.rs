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

use nix::errno::Errno;
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, unshare};
use std::fs::{File, create_dir_all, remove_dir, remove_file};
use std::io;
use std::path::PathBuf;
use tugboat_vm_runtime_interface::run::mount_namespace_path;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("System call error: {0}")]
    Syscall(#[from] Errno),
    #[error("Task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("Namespace path '{0}' has no parent directory")]
    NamespacePathHasNoParent(String),
    #[error("Mount namespace worker thread panicked")]
    ThreadPanic,
}

pub(crate) fn path_for_ship(ship_id: &str) -> PathBuf {
    PathBuf::from(mount_namespace_path(ship_id))
}

pub(crate) fn ensure_for_ship(ship_id: &str) -> Result<PathBuf, Error> {
    let namespace_path = path_for_ship(ship_id);
    if namespace_path.exists() {
        return Ok(namespace_path);
    }

    let Some(parent) = namespace_path.parent() else {
        return Err(Error::NamespacePathHasNoParent(
            namespace_path.display().to_string(),
        ));
    };
    create_dir_all(parent)?;

    let namespace_path_for_thread = namespace_path.clone();
    std::thread::spawn(move || -> Result<(), Error> {
        unshare(CloneFlags::CLONE_NEWNS)?;
        mount(
            None::<&str>,
            "/",
            None::<&str>,
            MsFlags::MS_REC | MsFlags::MS_PRIVATE,
            None::<&str>,
        )?;
        File::options()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&namespace_path_for_thread)?;
        match mount(
            Some("/proc/self/ns/mnt"),
            namespace_path_for_thread.as_path(),
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        ) {
            Ok(()) | Err(Errno::EBUSY) => Ok(()),
            Err(err) => Err(err.into()),
        }
    })
    .join()
    .map_err(|_| Error::ThreadPanic)??;

    Ok(namespace_path)
}

pub(crate) fn cleanup_for_ship(ship_id: &str) -> Result<(), Error> {
    let namespace_path = path_for_ship(ship_id);

    match umount2(&namespace_path, MntFlags::empty()) {
        Ok(()) => {}
        Err(Errno::EINVAL | Errno::ENOENT) => {}
        Err(err) => return Err(err.into()),
    }

    match remove_file(&namespace_path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    let Some(parent) = namespace_path.parent() else {
        return Err(Error::NamespacePathHasNoParent(
            namespace_path.display().to_string(),
        ));
    };
    match remove_dir(parent) {
        Ok(()) => {}
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(err) => return Err(err.into()),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::path_for_ship;

    #[test]
    fn can_plan_mount_namespace_path() {
        assert_eq!(
            path_for_ship("ship-uid").display().to_string(),
            "/var/run/tugboat/mntns/ship-uid"
        );
    }
}
