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

use crate::csi::PublishedVolume;
use crate::reconciler::ShipFingerprints;

pub(crate) struct Runtime {
    namespace: String,
    ship_name: String,
    id: String,
    fingerprints: ShipFingerprints,
    published_volumes: Vec<PublishedVolume>,
}

impl Runtime {
    pub(super) fn new(
        namespace: String,
        ship_name: String,
        id: String,
        fingerprints: ShipFingerprints,
        published_volumes: Vec<PublishedVolume>,
    ) -> Self {
        Self {
            namespace,
            ship_name,
            id,
            fingerprints,
            published_volumes,
        }
    }

    pub(super) fn into_published_volumes(self) -> Vec<PublishedVolume> {
        self.published_volumes
    }

    pub(super) fn matches_spec_fingerprint(&self, fingerprint: &str) -> bool {
        self.fingerprints.spec == fingerprint
    }

    pub(super) fn matches_pvc_volume_fingerprint(&self, fingerprint: &str) -> bool {
        self.fingerprints.pvc_volume == fingerprint
    }

    pub(super) fn matches_materialized_volume_fingerprint(&self, fingerprint: &str) -> bool {
        self.fingerprints.materialized_volume == fingerprint
    }

    pub(super) fn update_materialized_volume_fingerprint(&mut self, fingerprint: String) {
        self.fingerprints.materialized_volume = fingerprint;
    }
}
