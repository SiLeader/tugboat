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

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UserIdentity {
    Anonymous,
    ServiceAccount,
    X509,
    Oidc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UserInfo {
    pub(crate) username: String,
    pub(crate) uid: Option<String>,
    pub(crate) groups: Vec<String>,
    pub(crate) extra: HashMap<String, Vec<String>>,
    pub(crate) identity: UserIdentity,
}

impl UserInfo {
    pub(crate) fn anonymous() -> Self {
        Self {
            username: "system:anonymous".to_string(),
            uid: None,
            groups: vec!["system:unauthenticated".to_string()],
            extra: HashMap::new(),
            identity: UserIdentity::Anonymous,
        }
    }

    pub(crate) fn service_account(
        namespace: &str,
        name: &str,
        uid: Option<String>,
        extra: HashMap<String, Vec<String>>,
    ) -> Self {
        Self {
            username: format!("system:serviceaccount:{namespace}:{name}"),
            uid,
            groups: vec![
                "system:serviceaccounts".to_string(),
                format!("system:serviceaccounts:{namespace}"),
                "system:authenticated".to_string(),
            ],
            extra,
            identity: UserIdentity::ServiceAccount,
        }
    }

    pub(crate) fn x509(
        username: String,
        groups: Vec<String>,
        extra: HashMap<String, Vec<String>>,
    ) -> Self {
        let mut groups = groups;
        if !groups.iter().any(|group| group == "system:authenticated") {
            groups.push("system:authenticated".to_string());
        }
        Self {
            username,
            uid: None,
            groups,
            extra,
            identity: UserIdentity::X509,
        }
    }

    pub(crate) fn oidc(
        username: String,
        groups: Vec<String>,
        extra: HashMap<String, Vec<String>>,
    ) -> Self {
        let mut groups = groups;
        if !groups.iter().any(|group| group == "system:authenticated") {
            groups.push("system:authenticated".to_string());
        }
        Self {
            username,
            uid: None,
            groups,
            extra,
            identity: UserIdentity::Oidc,
        }
    }

    pub(crate) fn is_service_account(&self) -> bool {
        matches!(self.identity, UserIdentity::ServiceAccount)
    }
}
