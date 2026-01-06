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

use std::cmp::Ordering;

pub struct ResourceVersionRef<'a>(pub &'a str);

impl Eq for ResourceVersionRef<'_> {}

impl PartialEq for ResourceVersionRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl PartialOrd for ResourceVersionRef<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ResourceVersionRef<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.0.len() == other.0.len() {
            self.0.cmp(&other.0)
        } else if self.0.len() < other.0.len() {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::resource_version::ResourceVersionRef;

    #[test]
    fn can_compare_resource_version_1() {
        let a = ResourceVersionRef("1234567890");
        let b = ResourceVersionRef("1234567890");
        assert!(a == b);
    }

    #[test]
    fn can_compare_resource_version_2() {
        let a = ResourceVersionRef("2345678901234567890123456789012345678901");
        let b = ResourceVersionRef("345678901234567890123456789012345678901");
        assert!(a > b);
    }

    #[test]
    fn can_compare_resource_version_3() {
        let a = ResourceVersionRef("345678901234567890123456789012345678901");
        let b = ResourceVersionRef("345678901234567890123456789012345678901");
        assert!(a == b)
    }

    #[test]
    fn can_compare_resource_version_4() {
        let a = ResourceVersionRef("345678901234567890123456789012345678900");
        let b = ResourceVersionRef("345678901234567890123456789012345678901");
        assert!(a < b);
    }
}
