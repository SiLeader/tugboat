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

use tugboat_client::{Api, TugboatClient};
use tugboat_resources::manifests::coordination::v1::{Lease, LeaseSpec};
use tugboat_resources::manifests::meta::v1::Time;

const LEASE_NAME: &str = "tugboat-scheduler";
const LEASE_NAMESPACE: &str = "tugboat-system";

pub(crate) struct LeaderElector {
    lease_api: Api<Lease>,
    identity: String,
    lease_duration_seconds: u64,
    renew_interval_seconds: u64,
    is_leader: bool,
}

impl LeaderElector {
    pub fn new(
        client: TugboatClient,
        identity: String,
        lease_duration_seconds: u64,
        renew_interval_seconds: u64,
    ) -> Self {
        let lease_api = Api::namespaced(client, LEASE_NAMESPACE);
        Self {
            lease_api,
            identity,
            lease_duration_seconds,
            renew_interval_seconds,
            is_leader: false,
        }
    }

    pub fn is_leader(&self) -> bool {
        self.is_leader
    }

    pub fn renew_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.renew_interval_seconds)
    }

    pub async fn try_acquire_or_renew(&mut self) -> bool {
        match self.lease_api.get(LEASE_NAME).await {
            Ok(Some(lease)) => self.try_update_lease(lease).await,
            Ok(None) => self.try_create_lease().await,
            Err(e) => {
                tracing::warn!("Failed to get lease: {e}");
                self.is_leader = false;
                false
            }
        }
    }

    async fn try_create_lease(&mut self) -> bool {
        let now = Time::now();
        let lease = Lease {
            type_meta: None,
            object_meta: Some(tugboat_resources::manifests::meta::v1::ObjectMeta {
                name: Some(LEASE_NAME.to_string()),
                namespace: Some(LEASE_NAMESPACE.to_string()),
                ..Default::default()
            }),
            spec: Some(LeaseSpec {
                holder_identity: self.identity.clone(),
                lease_duration_seconds: self.lease_duration_seconds as i64,
                acquire_time: Some(now),
                renew_time: Some(now),
                lease_transitions: 0,
                preferred_holder: String::new(),
                strategy: String::new(),
            }),
        };

        match self.lease_api.create(lease).await {
            Ok(_) => {
                tracing::info!("Acquired leader lease");
                self.is_leader = true;
                true
            }
            Err(e) => {
                tracing::debug!("Failed to create lease (another scheduler may hold it): {e}");
                self.is_leader = false;
                false
            }
        }
    }

    async fn try_update_lease(&mut self, lease: Lease) -> bool {
        let spec = lease.spec.as_ref();
        let holder = spec.map(|s| s.holder_identity.as_str());
        let is_current_holder = holder == Some(self.identity.as_str());

        if is_current_holder {
            return self.renew_lease(lease).await;
        }

        // Check if lease is expired
        let renew_time = spec.and_then(|s| s.renew_time.as_ref());
        let duration = spec.map(|s| s.lease_duration_seconds).unwrap_or(15);

        if let Some(renew_time) = renew_time {
            let now = Time::now();
            let expiry_seconds = renew_time.seconds + duration;
            if now.seconds < expiry_seconds {
                // Lease is still valid, held by someone else
                if self.is_leader {
                    tracing::info!("Lost leader lease to {}", holder.unwrap_or("unknown"));
                }
                self.is_leader = false;
                return false;
            }
        }

        // Lease expired, try to acquire
        self.acquire_expired_lease(lease).await
    }

    async fn renew_lease(&mut self, lease: Lease) -> bool {
        // Use JSON patch or Merge patch if possible, but here we use replace_status equivalent logic
        // but since Lease spec is what we update, we need to be careful.
        // Actually, the issue might be that we are doing full replace.
        // Let's try to just update the renewTime in a loop with fresh fetching.

        let mut current_lease = lease;

        for _ in 0..5 {
            let now = Time::now();
            if let Some(ref mut spec) = current_lease.spec {
                spec.renew_time = Some(now);
            }

            match self
                .lease_api
                .replace(LEASE_NAME, current_lease.clone())
                .await
            {
                Ok(_) => {
                    self.is_leader = true;
                    return true;
                }
                Err(tugboat_client::Error::Api(status)) if status.code == 409 => {
                    tracing::warn!("Failed to renew lease due to conflict, retrying...");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                    match self.lease_api.get(LEASE_NAME).await {
                        Ok(Some(latest)) => {
                            let spec = latest.spec.as_ref();
                            let holder = spec.map(|s| s.holder_identity.as_str());
                            if holder == Some(self.identity.as_str()) {
                                current_lease = latest;
                                continue;
                            } else {
                                tracing::warn!("Lost leadership during renewal retry");
                                self.is_leader = false;
                                return false;
                            }
                        }
                        Ok(None) => {
                            self.is_leader = false;
                            return false;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to get lease during retry: {e}");
                            self.is_leader = false;
                            return false;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to renew lease: {e}");
                    self.is_leader = false;
                    return false;
                }
            }
        }

        tracing::warn!("Failed to renew lease after retries");
        self.is_leader = false;
        false
    }

    async fn acquire_expired_lease(&mut self, mut lease: Lease) -> bool {
        let now = Time::now();
        let transitions = lease
            .spec
            .as_ref()
            .map(|s| s.lease_transitions)
            .unwrap_or(0);

        if let Some(ref mut spec) = lease.spec {
            spec.holder_identity = self.identity.clone();
            spec.acquire_time = Some(now);
            spec.renew_time = Some(now);
            spec.lease_transitions = transitions + 1;
            spec.lease_duration_seconds = self.lease_duration_seconds as i64;
        }

        match self.lease_api.replace(LEASE_NAME, lease).await {
            Ok(_) => {
                tracing::info!("Acquired expired leader lease");
                self.is_leader = true;
                true
            }
            Err(e) => {
                tracing::debug!("Failed to acquire expired lease: {e}");
                self.is_leader = false;
                false
            }
        }
    }
}
