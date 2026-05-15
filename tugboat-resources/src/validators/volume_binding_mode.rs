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

use crate::validators::Validator;

const VALID_VOLUME_BINDING_MODES: &[&str] = &["Immediate", "WaitForFirstConsumer"];

pub trait HasVolumeBindingMode {
    fn volume_binding_mode_value(&self) -> Option<&str>;
}

pub struct VolumeBindingModeValidator;

impl<T> Validator<T> for VolumeBindingModeValidator
where
    T: HasVolumeBindingMode,
{
    fn validate(&self, value: &T) -> bool {
        match value.volume_binding_mode_value() {
            None => true,
            Some(mode) => VALID_VOLUME_BINDING_MODES.contains(&mode),
        }
    }
}
