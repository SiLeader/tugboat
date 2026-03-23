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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackoffConfig {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub factor: u32,
}

impl BackoffConfig {
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let mut millis = self.initial_delay.as_millis().max(1);
        let max_millis = self.max_delay.as_millis().max(1);
        let factor = u128::from(self.factor.max(1));

        for _ in 0..attempt {
            millis = millis.saturating_mul(factor).min(max_millis);
            if millis == max_millis {
                break;
            }
        }

        Duration::from_millis(millis.min(u128::from(u64::MAX)) as u64)
    }
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(128),
            factor: 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BackoffConfig;
    use std::time::Duration;

    #[test]
    fn caps_backoff_at_maximum() {
        let config = BackoffConfig {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(5),
            factor: 3,
        };

        assert_eq!(config.delay_for(0), Duration::from_secs(1));
        assert_eq!(config.delay_for(1), Duration::from_secs(3));
        assert_eq!(config.delay_for(2), Duration::from_secs(5));
        assert_eq!(config.delay_for(10), Duration::from_secs(5));
    }
}
