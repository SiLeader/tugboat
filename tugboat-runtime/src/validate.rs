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

/// Validates that an identifier (VM ID, ship ID, container ID) contains only
/// safe characters and cannot be used for path traversal.
///
/// Allowed characters: alphanumeric, hyphen (`-`), underscore (`_`), and dot (`.`).
/// The identifier must not be empty, start with a dot, or contain `..`.
pub(crate) fn validate_safe_id(id: &str, field_name: &str) -> Result<(), Error> {
    if id.is_empty() {
        return Err(Error::Validation(format!("{field_name} must not be empty")));
    }
    if id.starts_with('.') {
        return Err(Error::Validation(format!(
            "{field_name} must not start with '.'"
        )));
    }
    if id.contains("..") {
        return Err(Error::Validation(format!(
            "{field_name} must not contain '..'"
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(Error::Validation(format!(
            "{field_name} contains invalid characters; only alphanumeric, '-', '_', and '.' are allowed"
        )));
    }
    Ok(())
}

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
    fn valid_ids_are_accepted() {
        assert!(validate_safe_id("my-vm-123", "id").is_ok());
        assert!(validate_safe_id("ship_abc.456", "id").is_ok());
        assert!(validate_safe_id("a", "id").is_ok());
    }

    #[test]
    fn empty_id_is_rejected() {
        assert!(validate_safe_id("", "id").is_err());
    }

    #[test]
    fn path_traversal_is_rejected() {
        assert!(validate_safe_id("../etc/passwd", "id").is_err());
        assert!(validate_safe_id("foo/../bar", "id").is_err());
        assert!(validate_safe_id("/absolute", "id").is_err());
    }

    #[test]
    fn dot_prefix_is_rejected() {
        assert!(validate_safe_id(".hidden", "id").is_err());
    }

    #[test]
    fn special_characters_are_rejected() {
        assert!(validate_safe_id("id with spaces", "id").is_err());
        assert!(validate_safe_id("id;rm -rf /", "id").is_err());
    }

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
