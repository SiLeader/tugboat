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

use qapi::futures::{QapiStream, QmpStreamTokio};
use std::future::Future;
use std::path::Path;
use std::time::Duration;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::UnixStream;
use tokio::time::timeout;

pub(crate) const QMP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const QMP_NEGOTIATE_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const QMP_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn connect_qmp(
    uds_path: impl AsRef<Path>,
) -> crate::Result<
    QapiStream<QmpStreamTokio<ReadHalf<UnixStream>>, QmpStreamTokio<WriteHalf<UnixStream>>>,
> {
    let stream = timeout(QMP_CONNECT_TIMEOUT, QmpStreamTokio::open_uds(uds_path))
        .await
        .map_err(|_| crate::Error::Qmp("Timed out connecting to QMP socket".to_string()))?
        .map_err(|e| crate::Error::Qmp(e.to_string()))?;

    timeout(QMP_NEGOTIATE_TIMEOUT, stream.negotiate())
        .await
        .map_err(|_| crate::Error::Qmp("Timed out negotiating QMP capabilities".to_string()))?
        .map_err(|e| crate::Error::Qmp(e.to_string()))
}

pub(crate) async fn execute_with_timeout<T, E>(
    operation: impl Future<Output = Result<T, E>>,
    timeout_message: &str,
) -> crate::Result<T>
where
    E: std::fmt::Display,
{
    timeout(QMP_COMMAND_TIMEOUT, operation)
        .await
        .map_err(|_| crate::Error::Qmp(timeout_message.to_string()))?
        .map_err(|e| crate::Error::Qmp(e.to_string()))
}
