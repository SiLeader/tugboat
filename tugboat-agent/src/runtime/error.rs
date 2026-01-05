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

use std::process::ExitStatus;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum RuntimeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Missing field: {0}")]
    MissingField(String),
    #[error("Image error: {0}")]
    Image(#[from] tugboat_vm_image::Error),
    #[error("JSON Serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Run VM error")]
    RunVm,
    #[error("Invalid memory size: {0}")]
    MemorySize(String),
    #[error("Failed to execute command: status: {0}, stdout: '{1}', stderr: '{2}'")]
    CommandFailed(ExitStatus, String, String),
}
