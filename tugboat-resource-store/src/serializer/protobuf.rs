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

use crate::error::Error;
use crate::serializer::Serializer;
use tugboat_resources::manifests::meta::v1::TypeMeta;

#[derive(Default)]
pub struct ProtobufSerializer;

impl Serializer for ProtobufSerializer {
    fn is_supported(&self, type_meta: &TypeMeta) -> bool {
        !type_meta
            .api_version
            .as_ref()
            .is_some_and(|v| v.contains("/")) // core
    }

    fn serialize_protobuf<T: prost::Message>(
        &self,
        type_meta: &TypeMeta,
        value: &T,
    ) -> Result<Vec<u8>, Error> {
        if !self.is_supported(type_meta) {
            return Err(Error::UnsupportedType);
        }
        Ok(value.encode_to_vec())
    }

    fn deserialize_protobuf<T>(&self, type_meta: &TypeMeta, data: &[u8]) -> Result<T, Error>
    where
        T: prost::Message + Default,
    {
        if !self.is_supported(type_meta) {
            return Err(Error::UnsupportedType);
        }
        Ok(T::decode(data)?)
    }
}
