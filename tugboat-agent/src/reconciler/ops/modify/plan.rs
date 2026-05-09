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

use super::super::ShipFingerprints;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ModifyPlan {
    Unchanged,
    RuntimeReconfigure {
        spec_changed: bool,
        pvc_changed: bool,
    },
    RefreshMaterializedVolumes {
        materialized_volume_fingerprint: String,
    },
}

pub(super) fn plan_modify_action(
    spec_changed: bool,
    pvc_changed: bool,
    materialized_volume_changed: bool,
    fingerprints: &ShipFingerprints,
) -> ModifyPlan {
    if spec_changed || pvc_changed {
        return ModifyPlan::RuntimeReconfigure {
            spec_changed,
            pvc_changed,
        };
    }

    if materialized_volume_changed {
        return ModifyPlan::RefreshMaterializedVolumes {
            materialized_volume_fingerprint: fingerprints.materialized_volume.clone(),
        };
    }

    ModifyPlan::Unchanged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprints() -> ShipFingerprints {
        ShipFingerprints {
            spec: "spec".to_string(),
            pvc_volume: "pvc".to_string(),
            materialized_volume: "mat".to_string(),
        }
    }

    #[test]
    fn unchanged_plan_when_no_runtime_relevant_diff_exists() {
        assert_eq!(
            plan_modify_action(false, false, false, &fingerprints()),
            ModifyPlan::Unchanged
        );
    }

    #[test]
    fn runtime_reconfigure_takes_precedence_over_materialized_refresh() {
        assert_eq!(
            plan_modify_action(true, false, true, &fingerprints()),
            ModifyPlan::RuntimeReconfigure {
                spec_changed: true,
                pvc_changed: false,
            }
        );
    }

    #[test]
    fn materialized_only_change_keeps_refresh_fingerprint() {
        assert_eq!(
            plan_modify_action(false, false, true, &fingerprints()),
            ModifyPlan::RefreshMaterializedVolumes {
                materialized_volume_fingerprint: "mat".to_string(),
            }
        );
    }
}
