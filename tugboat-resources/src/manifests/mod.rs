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

fn default<T: Default + PartialEq>(t: &T) -> bool {
    *t == Default::default()
}

pub mod core {
    pub mod v1 {
        use crate::validators::{NameValidator, NamespaceProhibitedValidator};
        use crate::{apply_resource, apply_validators};

        include!(concat!(env!("OUT_DIR"), "/tugboat.core.v1.rs"));

        apply_resource!(Namespace, "core", "v1", "namespaces", "namespace", cluster);
        apply_resource!(
            NetworkClass,
            "core",
            "v1",
            "networkclasses",
            "networkclass",
            namespaced
        );
        apply_resource!(
            ClusterNetworkClass,
            "core",
            "v1",
            "clusternetworkclasses",
            "clusternetworkclass",
            cluster
        );
        apply_resource!(Node, "core", "v1", "nodes", "node", cluster);
        apply_resource!(Ship, "core", "v1", "ships", "ship", namespaced);
        apply_resource!(ShipClass, "core", "v1", "shipclasses", "shipclass", cluster);

        apply_validators!(Namespace, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(NetworkClass, validators NameValidator);
        apply_validators!(ClusterNetworkClass, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(Node, validators NameValidator, NamespaceProhibitedValidator);
        apply_validators!(Ship, validators NameValidator);
        apply_validators!(ShipClass, validators NameValidator, NamespaceProhibitedValidator);
    }
}

pub mod coordination {
    pub mod v1 {
        use crate::validators::NameValidator;
        use crate::{apply_resource, apply_validators};

        include!(concat!(env!("OUT_DIR"), "/tugboat.coordination.v1.rs"));

        apply_resource!(Lease, "coordination", "v1", "leases", "lease", namespaced);

        apply_validators!(Lease, validators NameValidator);
    }
}

pub mod meta {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/tugboat.meta.v1.rs"));

        impl Time {
            pub fn now() -> Self {
                let now = std::time::SystemTime::now();
                let duration = now
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();

                let seconds = duration.as_secs() as i64;
                let nanos = duration.subsec_nanos() as i32;
                Time { seconds, nanos }
            }
        }

        impl serde::Serialize for Time {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                if let Some(dt) =
                    ::chrono::DateTime::from_timestamp(self.seconds, self.nanos as u32)
                {
                    serializer.serialize_str(&dt.to_rfc3339())
                } else {
                    use serde::ser::Error;
                    Err(S::Error::custom("invalid timestamp"))
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for Time {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                let dt =
                    ::chrono::DateTime::parse_from_rfc3339(&s).map_err(serde::de::Error::custom)?;
                let dt_utc = dt.with_timezone(&::chrono::Utc);
                Ok(Time {
                    seconds: dt_utc.timestamp(),
                    nanos: dt_utc.timestamp_subsec_nanos() as i32,
                })
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn test_time_serialization() {
                let t = Time {
                    seconds: 1678896000, // 2023-03-15T16:00:00Z
                    nanos: 0,
                };
                let json = serde_json::to_string(&t).unwrap();
                // Depending on chrono version/timezone, output might vary slightly, but should be RFC3339
                assert!(json.contains("2023-03-15T16:00:00"));

                let t2: Time = serde_json::from_str(&json).unwrap();
                assert_eq!(t.seconds, t2.seconds);
                assert_eq!(t.nanos, t2.nanos);
            }

            #[test]
            fn test_time_deserialization() {
                let json = "\"2023-03-15T16:00:00.123456Z\"";
                let t: Time = serde_json::from_str(json).unwrap();
                assert_eq!(t.seconds, 1678896000);
                assert_eq!(t.nanos, 123456000);
            }
        }
    }
}
