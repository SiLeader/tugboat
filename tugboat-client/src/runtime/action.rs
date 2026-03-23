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

use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Action {
    requeue_after: Option<Duration>,
}

impl Action {
    pub fn await_change() -> Self {
        Self {
            requeue_after: None,
        }
    }

    pub fn requeue(duration: Duration) -> Self {
        Self {
            requeue_after: Some(duration),
        }
    }

    pub fn requeue_immediately() -> Self {
        Self::requeue(Duration::ZERO)
    }

    pub fn requeue_after(&self) -> Option<Duration> {
        self.requeue_after
    }
}
