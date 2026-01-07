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

use nix::sched::{CloneFlags, setns};
use std::fs::File;

pub(crate) fn enter_to_network_namespace(network_namespace: &str) -> Result<(), crate::Error> {
    let netns_path = format!("/var/run/netns/{network_namespace}");
    let netns_file = File::open(netns_path)?;
    setns(netns_file, CloneFlags::CLONE_NEWNET)?;

    Ok(())
}

pub(crate) async fn change_running_user_and_group(
    user: &str,
    group: &str,
) -> Result<(), crate::Error> {
    todo!()
}
