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

pub(crate) mod add;
pub(crate) mod delete;
pub(crate) mod modify;

/// Compute a stable SHA-256 fingerprint for a serializable spec value.
/// Uses JSON as the canonical byte representation and returns a lowercase hex string.
pub(super) fn spec_fingerprint<T: serde::Serialize>(
    spec: &T,
) -> Result<String, serde_json::Error> {
    use sha2::Digest;
    let json = serde_json::to_string(spec)?;
    let hash = sha2::Sha256::digest(json.as_bytes());
    Ok(format!("{hash:x}"))
}
