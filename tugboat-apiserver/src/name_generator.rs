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

use rand::SeedableRng;
use rand::distr::{Alphanumeric, SampleString};
use rand_xorshift::XorShiftRng;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) struct NameGenerator {
    rand: Arc<Mutex<XorShiftRng>>,
}

impl NameGenerator {
    pub(crate) fn new() -> Self {
        Self {
            rand: Arc::new(Mutex::new(XorShiftRng::from_os_rng())),
        }
    }

    pub(crate) async fn generate_with_len(&self, name_base: &str, length: usize) -> String {
        let mut rng = self.rand.lock().await;
        format!(
            "{name_base}{}",
            Alphanumeric.sample_string(&mut rng, length)
        )
    }

    pub(crate) async fn generate(&self, name_base: &str) -> String {
        self.generate_with_len(name_base, 5).await
    }
}
