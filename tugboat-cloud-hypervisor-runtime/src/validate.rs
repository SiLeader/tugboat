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

use crate::Error;

/// Validates values interpolated into Cloud Hypervisor key=value option strings.
pub(crate) fn validate_ch_option_value(value: &str, field_name: &str) -> Result<(), Error> {
    if value.contains(',') || value.contains('=') {
        return Err(Error::Validation(format!(
            "{field_name} must not contain ',' or '='"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_hypervisor_option_values_reject_delimiters() {
        assert!(validate_ch_option_value("/tmp/disk.img", "path").is_ok());
        assert!(validate_ch_option_value("tag,value", "tag").is_err());
        assert!(validate_ch_option_value("tag=value", "tag").is_err());
    }
}
