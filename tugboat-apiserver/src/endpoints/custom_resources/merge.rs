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

use crate::data::StatusResponse;

pub(super) fn rfc7396_merge_patch(target: &mut serde_json::Value, patch: &serde_json::Value) {
    match patch {
        serde_json::Value::Object(patch_map) => {
            let target_map = match target {
                serde_json::Value::Object(map) => map,
                _ => {
                    *target = serde_json::Value::Object(serde_json::Map::new());
                    match target {
                        serde_json::Value::Object(map) => map,
                        _ => unreachable!("target was just set to object"),
                    }
                }
            };
            for (key, value) in patch_map {
                if value.is_null() {
                    target_map.remove(key);
                } else {
                    rfc7396_merge_patch(
                        target_map.entry(key).or_insert(serde_json::Value::Null),
                        value,
                    );
                }
            }
        }
        _ => {
            *target = patch.clone();
        }
    }
}

pub(super) fn patch_object(
    patch: serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, Box<StatusResponse>> {
    match patch {
        serde_json::Value::Object(map) => Ok(map),
        _ => Err(Box::new(StatusResponse::bad_request(
            "JSON merge patch body must be an object",
            None,
        ))),
    }
}
