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

use clap::Parser;

mod config;
mod reconciler;
mod runtime;

#[derive(Debug, Parser)]
struct Args {
    #[arg(
        long,
        help = "Path to the tugboat-agent config file",
        default_value = "/etc/tugboat/agent/config.toml"
    )]
    config: String,
}

pub async fn run() {
    let args = Args::parse();
    let config = config::AgentConfig::load_or_panic(args.config);
}
