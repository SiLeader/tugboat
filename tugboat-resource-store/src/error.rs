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

use crate::watch::WatchEvent;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unsupported type")]
    UnsupportedType,
    #[error("Protobuf serialization error: {0}")]
    ProtobufDeserialization(#[from] prost::DecodeError),
    #[error("Required field missing: field: '{0}'")]
    FieldMissing(String),
    #[error("Etcd error: {0}")]
    Etcd(#[from] etcd_client::Error),
    #[error("Event emit error: {0}")]
    EventEmit(#[from] tokio::sync::watch::error::SendError<Vec<WatchEvent>>),
    #[error("Optimistic lock error: revision: {0}")]
    OptimisticLockFailed(i64),
    #[error("Invalid resource version: {0}")]
    InvalidResourceVersion(String),
}
