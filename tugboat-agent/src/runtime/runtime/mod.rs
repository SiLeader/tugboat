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

mod status;

use tokio::process::Child;

pub(crate) struct Runtime {
    namespace: String,
    id: String,
    child: Child,
}

impl Runtime {
    pub(super) fn new(namespace: String, id: String, child: Child) -> Self {
        Self {
            namespace,
            id,
            child,
        }
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn namespace(&self) -> &str {
        &self.namespace
    }
}
