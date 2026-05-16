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

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/tugboat.meta.v1.rs"));

    impl crate::Resource for CustomResourceObject {
        fn type_meta() -> TypeMeta {
            TypeMeta {
                api_version: Some("meta/v1".to_string()),
                kind: Some("CustomResourceObject".to_string()),
            }
        }
    }

    impl crate::ObjectMetaResource for CustomResourceObject {
        fn object_meta(&self) -> &Option<ObjectMeta> {
            &self.object_meta
        }

        fn object_meta_mut(&mut self) -> &mut Option<ObjectMeta> {
            &mut self.object_meta
        }
    }

    impl crate::SetTypeMeta for CustomResourceObject {
        fn set_type_meta(&mut self, type_meta: TypeMeta) {
            self.type_meta = Some(type_meta);
        }
    }

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
            if let Some(dt) = ::chrono::DateTime::from_timestamp(self.seconds, self.nanos as u32) {
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
