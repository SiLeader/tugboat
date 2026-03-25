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
        match ::tugboat_resources::ObjectMetaResource::object_meta(&$obj).clone() {
            Some(meta) => meta,
            None => {
                return Err(Box::new($crate::data::StatusResponse::bad_request(
                    "metadata is required",
                    None,
                )))
            }
        }
    };
}

#[macro_export]
macro_rules! check_namespace_absent {
    ($object_meta:expr) => {
        if $object_meta.namespace.is_some() {
            return Err(Box::new($crate::data::StatusResponse::bad_request(
                "metadata.namespace cannot be set",
                None,
            )));
        }
    };
}

#[macro_export]
macro_rules! create_object {
    ($operator:expr, $object_meta:expr, $object:expr, $type_meta:expr) => {{
        let mut object = $object;
        ::tugboat_resources::SetTypeMeta::set_type_meta(&mut object, $type_meta);

        let object_meta = $operator.apply_uid($object_meta);

        if object_meta.name.is_some() {
            ::tugboat_resources::ObjectMetaResource::set_object_meta(
                &mut object,
                Some(object_meta),
            );
            return if let Some(data) = $operator
                .store
                .put_if_not_exists(object)
                .await
                .map_err(|e| Box::new(e.into()))?
            {
                Ok($crate::data::ModifyResponse::Created(data.apply_revision()))
            } else {
                Err(Box::new(StatusResponse::conflict(
                    "Specified name is already exists",
                    None,
                )))
            };
        }

        let Some(generate_name) = &object_meta.generate_name else {
            return Err(Box::new(StatusResponse::bad_request(
                "metadata.generateName is required",
                None,
            )));
        };
        for _ in 0..5 {
            let name = $operator.name_generator.generate(generate_name).await;
            let object = {
                let mut obj = object.clone();
                ::tugboat_resources::ObjectMetaResource::set_object_meta(
                    &mut obj,
                    Some(tugboat_resources::manifests::meta::v1::ObjectMeta {
                        name: Some(name),
                        ..object_meta.clone()
                    }),
                );
                obj
            };
            if let Some(data) = $operator
                .store
                .put_if_not_exists(object)
                .await
                .map_err(|e| Box::new(e.into()))?
            {
                return Ok($crate::data::ModifyResponse::Created(data.apply_revision()));
            }
        }
        Err(Box::new(StatusResponse::conflict(
            "Generate name failed",
            None,
        )))
    }};
}
