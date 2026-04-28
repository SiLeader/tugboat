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

/// Validates that a value intended for a QEMU option string does not contain
/// characters that QEMU interprets as delimiters (comma, equals).
/// This prevents injection of additional options via user-controlled fields.
pub(crate) fn validate_qemu_option_value(value: &str, field_name: &str) -> Result<(), Error> {
    if value.contains(',') || value.contains('=') {
        return Err(Error::Validation(format!(
            "{field_name} must not contain ',' or '=' (QEMU option delimiter)"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_qemu_values_are_accepted() {
        assert!(validate_qemu_option_value("tap0", "iface").is_ok());
        assert!(validate_qemu_option_value("aa:bb:cc:dd:ee:ff", "mac").is_ok());
        assert!(validate_qemu_option_value("/var/lib/disk.qcow2", "path").is_ok());
    }

    #[test]
    fn qemu_comma_injection_is_rejected() {
        assert!(validate_qemu_option_value("tap0,script=/tmp/evil.sh", "iface").is_err());
    }

    #[test]
    fn qemu_equals_injection_is_rejected() {
        assert!(validate_qemu_option_value("tap0=bad", "iface").is_err());
    }
}
