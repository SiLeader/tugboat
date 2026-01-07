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

use std::fs::File;
use tugboat_vm_runtime_interface::run::VmRunRequest;

pub(super) fn load_run_config_or_panic(path: String) -> VmRunRequest {
    if path == "-" {
        serde_json::from_reader(std::io::stdin())
    } else {
        serde_json::from_reader(File::open(path).expect("Cannot open config file"))
    }
    .expect("Failed to parse VM config file as TOML")
}
