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
use prost::Message;
use tugboat_resources::manifests::meta::v1::CustomResourceObject;

pub trait CustomResourceSerializable: Sized {
    fn serialize(&self) -> Result<Vec<u8>, Error>;
    fn deserialize(data: &[u8]) -> Result<Self, Error>;
}

impl CustomResourceSerializable for CustomResourceObject {
    fn serialize(&self) -> Result<Vec<u8>, Error> {
        Ok(self.encode_to_vec())
    }

    fn deserialize(data: &[u8]) -> Result<Self, Error> {
        Ok(Self::decode(data)?)
    }
}
