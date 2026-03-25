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

mod name;
mod namespace;
mod reclaim_policy;

pub use name::*;
pub use namespace::*;
pub use reclaim_policy::*;

pub trait Validator<T> {
    fn validate(&self, value: &T) -> bool;
}

pub trait Validatable {
    fn validate(&self) -> bool;
}

#[macro_export]
macro_rules! apply_validators {
    ($ty:ident, validators $($validator:expr),+ $(,)?) => {
        impl $crate::validators::Validatable for $ty {
            fn validate(&self) -> bool {
                $($crate::validators::Validator::validate(&$validator, self) &&)+
                true
            }
        }
    };
}
