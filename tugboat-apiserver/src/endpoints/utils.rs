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

#[macro_export]
macro_rules! extract_object_meta {
    ($obj:expr) => {
        match $obj.object_meta.clone() {
            Some(meta) => meta,
            None => {
                return Err($crate::data::StatusResponse::bad_request(
                    "metadata is required",
                    None,
                ))
            }
        }
    };
}

#[macro_export]
macro_rules! check_namespace_absent {
    ($object_meta:expr) => {
        if $object_meta.namespace.is_some() {
            return Err($crate::data::StatusResponse::bad_request(
                "metadata.namespace cannot be set",
                None,
            ));
        }
    };
}

#[macro_export]
macro_rules! create_object {
    ($operator:expr, $object_meta:expr, $object:expr, $type_meta:expr) => {{
        let mut object = $object;
        object.type_meta = Some($type_meta);

        for _ in 0..5 {
            let name = match $object_meta.name.clone() {
                None => match &$object_meta.generate_name {
                    None => {
                        return Err(StatusResponse::bad_request(
                            "metadata.name or metadata.generateName is required",
                            None,
                        ));
                    }
                    Some(base_name) => $operator.name_generator.generate(&base_name).await,
                },
                Some(name) => name,
            };

            let object = {
                let mut obj = object.clone();
                obj.object_meta = Some(tugboat_resources::manifests::meta::v1::ObjectMeta {
                    name: Some(name),
                    ..$object_meta.clone()
                });
                obj
            };
            if let Some(data) = $operator.store.put_if_not_exists(object).await? {
                return Ok($crate::data::ModifyResponse::Created(data.apply_revision()));
            }
        }

        Err(StatusResponse::conflict("Generate name failed", None))
    }};
}

#[macro_export]
macro_rules! check_conflict_optimistic {
    ($current:expr, $replacement:expr) => {{
        if let Some(current_meta) = &$current.object_meta
            && let Some(replacement_meta) = &$replacement.object_meta
        {
            if let Some(current_resource_version) = &current_meta.resource_version
                && let Some(replacement_resource_version) = &replacement_meta.resource_version
            {
                if current_resource_version != replacement_resource_version {
                    return Err($crate::data::StatusResponse::conflict(
                        "'resourceVersion' does not match.",
                        Some(::serde_json::json!({"current": current_resource_version})),
                    ));
                }
            }
        }
    }};
}
