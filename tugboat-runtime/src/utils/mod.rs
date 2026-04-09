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

use crate::Error;
use crate::validate::validate_safe_id;
use nix::libc::{S_IRUSR, S_IWUSR};
use nix::sys::stat::Mode;
use std::fs::File;
use std::io::{Read, Write};
use tracing::{debug, info};

fn create_fifo_path(container_id: &str) -> Result<String, Error> {
    validate_safe_id(container_id, "container_id")?;
    Ok(format!("/tmp/tugboat-runtime-{container_id}-exec.fifo"))
}

pub(crate) fn create_signal_fifo(container_id: &str) -> Result<(), Error> {
    debug!("Create fifo for signal. id = {container_id}");
    let path = create_fifo_path(container_id)?;

    nix::unistd::mkfifo(path.as_str(), Mode::from_bits_truncate(S_IRUSR | S_IWUSR))?;
    info!("FIFO for signal created. path = {path:?}");
    Ok(())
}

pub(crate) fn start_signal_using_fifo(container_id: &str) -> Result<(), Error> {
    debug!("Sending signal using fifo. id = {container_id}");
    let path = create_fifo_path(container_id)?;

    let mut file = File::options()
        .write(true)
        .create_new(false)
        .read(false)
        .open(&path)?;
    file.write_all(&[0u8])?;
    file.flush()?;

    info!("Successfully sent signal using fifo. path = {path}");
    Ok(())
}

pub(crate) fn wait_signal_using_fifo(container_id: &str) -> Result<(), Error> {
    debug!("Waiting signal using fifo. id = {container_id}");
    let path = create_fifo_path(container_id)?;

    info!("Waiting signal using fifo. path = {path}");
    let mut file = File::options()
        .read(true)
        .write(false)
        .create_new(false)
        .open(&path)?;
    let mut buf = [0u8; 1];
    file.read_exact(&mut buf)?;
    info!("Received signal using fifo. path = {path}");

    // Clean up the FIFO after successfully receiving the signal
    if let Err(e) = std::fs::remove_file(&path) {
        tracing::warn!("Failed to remove FIFO '{path}': {e}");
    }

    Ok(())
}
