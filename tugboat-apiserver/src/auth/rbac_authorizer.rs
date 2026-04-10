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

#![allow(dead_code)]

use crate::auth::authorization::{AuthorizationDecision, AuthorizationRequest, Authorizer};
use crate::auth::user_info::UserInfo;
use async_trait::async_trait;
use tugboat_resource_store::ResourceStore;
use tugboat_resources::manifests::authorization::v1::{
    ClusterRole, ClusterRoleBinding, PolicyRule, Role, RoleBinding, RoleRef, Subject,
};

const RBAC_API_GROUP: &str = "authorization";
const SYSTEM_MASTERS_GROUP: &str = "system:masters";
const USER_SUBJECT_KIND: &str = "User";
const GROUP_SUBJECT_KIND: &str = "Group";
const SERVICE_ACCOUNT_SUBJECT_KIND: &str = "ServiceAccount";
const ROLE_KIND: &str = "Role";
const CLUSTER_ROLE_KIND: &str = "ClusterRole";

pub(crate) struct RbacAuthorizer {
    store: ResourceStore,
}

impl RbacAuthorizer {
    pub(crate) fn new(store: ResourceStore) -> Self {
        Self { store }
    }

    pub(crate) async fn authorize_with_store(
        store: &ResourceStore,
        request: &AuthorizationRequest,
    ) -> AuthorizationDecision {
        match Self::authorize_impl_with_store(store, request).await {
            Ok(decision) => decision,
            Err(err) => AuthorizationDecision::Denied {
                reason: format!("Failed to evaluate RBAC policy: {err}"),
            },
        }
    }

    async fn authorize_impl(
        &self,
        request: &AuthorizationRequest,
    ) -> Result<AuthorizationDecision, tugboat_resource_store::error::Error> {
        Self::authorize_impl_with_store(&self.store, request).await
    }

    async fn authorize_impl_with_store(
        store: &ResourceStore,
        request: &AuthorizationRequest,
    ) -> Result<AuthorizationDecision, tugboat_resource_store::error::Error> {
        if is_superuser(&request.user) {
            return Ok(AuthorizationDecision::Allowed);
        }

        let mut rules = Self::cluster_role_rules(store, &request.user).await?;
        if let Some(namespace) = request.namespace.as_deref() {
            rules.extend(Self::role_binding_rules(store, &request.user, namespace).await?);
        }

        if rules.iter().any(|rule| rule_matches_request(rule, request)) {
            return Ok(AuthorizationDecision::Allowed);
        }

        Ok(AuthorizationDecision::Denied {
            reason: format!(
                "RBAC denied {} access to {}/{}",
                request.verb, request.api_group, request.resource
            ),
        })
    }

    async fn cluster_role_rules(
        store: &ResourceStore,
        user: &UserInfo,
    ) -> Result<Vec<PolicyRule>, tugboat_resource_store::error::Error> {
        let bindings = store.list::<ClusterRoleBinding>(None, None).await?;
        let mut rules = Vec::new();

        for binding in bindings.into_iter().map(|binding| binding.apply_revision()) {
            if !subjects_match_user(&binding.subjects, user) {
                continue;
            }
            rules.extend(Self::resolve_role_ref(store, binding.role_ref.as_ref(), None).await?);
        }

        Ok(rules)
    }

    async fn role_binding_rules(
        store: &ResourceStore,
        user: &UserInfo,
        namespace: &str,
    ) -> Result<Vec<PolicyRule>, tugboat_resource_store::error::Error> {
        let bindings = store
            .list::<RoleBinding>(Some(namespace.to_string()), None)
            .await?;
        let mut rules = Vec::new();

        for binding in bindings.into_iter().map(|binding| binding.apply_revision()) {
            if !subjects_match_user(&binding.subjects, user) {
                continue;
            }
            rules.extend(
                Self::resolve_role_ref(store, binding.role_ref.as_ref(), Some(namespace)).await?,
            );
        }

        Ok(rules)
    }

    async fn resolve_role_ref(
        store: &ResourceStore,
        role_ref: Option<&RoleRef>,
        namespace: Option<&str>,
    ) -> Result<Vec<PolicyRule>, tugboat_resource_store::error::Error> {
        let Some(role_ref) = role_ref else {
            return Ok(Vec::new());
        };
        if !role_ref.api_group.is_empty() && role_ref.api_group != RBAC_API_GROUP {
            return Ok(Vec::new());
        }

        match role_ref.kind.as_str() {
            ROLE_KIND => {
                let Some(namespace) = namespace else {
                    return Ok(Vec::new());
                };
                let role = store
                    .get::<Role>(Some(namespace.to_string()), &role_ref.name)
                    .await?;
                Ok(role
                    .map(|role| role.apply_revision().rules)
                    .unwrap_or_default())
            }
            CLUSTER_ROLE_KIND => {
                let role = store.get::<ClusterRole>(None, &role_ref.name).await?;
                Ok(role
                    .map(|role| role.apply_revision().rules)
                    .unwrap_or_default())
            }
            _ => Ok(Vec::new()),
        }
    }
}

#[async_trait]
impl Authorizer for RbacAuthorizer {
    async fn authorize(&self, request: &AuthorizationRequest) -> AuthorizationDecision {
        match self.authorize_impl(request).await {
            Ok(decision) => decision,
            Err(err) => AuthorizationDecision::Denied {
                reason: format!("Failed to evaluate RBAC policy: {err}"),
            },
        }
    }
}

fn is_superuser(user: &UserInfo) -> bool {
    user.groups
        .iter()
        .any(|group| group == SYSTEM_MASTERS_GROUP)
}

fn subjects_match_user(subjects: &[Subject], user: &UserInfo) -> bool {
    subjects
        .iter()
        .any(|subject| subject_matches_user(subject, user))
}

fn subject_matches_user(subject: &Subject, user: &UserInfo) -> bool {
    match subject.kind.as_str() {
        USER_SUBJECT_KIND => subject.name == user.username,
        GROUP_SUBJECT_KIND => user.groups.iter().any(|group| group == &subject.name),
        SERVICE_ACCOUNT_SUBJECT_KIND => service_account_subject_matches_user(subject, user),
        _ => false,
    }
}

fn service_account_subject_matches_user(subject: &Subject, user: &UserInfo) -> bool {
    let Some((namespace, name)) = parse_service_account_username(&user.username) else {
        return false;
    };
    subject.namespace.as_deref() == Some(namespace) && subject.name == name
}

fn parse_service_account_username(username: &str) -> Option<(&str, &str)> {
    let rest = username.strip_prefix("system:serviceaccount:")?;
    let (namespace, name) = rest.split_once(':')?;
    Some((namespace, name))
}

fn rule_matches_request(rule: &PolicyRule, request: &AuthorizationRequest) -> bool {
    matches_value(&rule.api_groups, request.api_group.as_str())
        && matches_value(&rule.resources, request.resource.as_str())
        && matches_value(&rule.verbs, request.verb.as_str())
        && matches_resource_name(&rule.resource_names, request.resource_name.as_deref())
}

fn matches_value(values: &[String], requested: &str) -> bool {
    values
        .iter()
        .any(|value| value == "*" || value == requested)
}

fn matches_resource_name(resource_names: &[String], requested: Option<&str>) -> bool {
    if resource_names.is_empty() {
        return true;
    }
    requested.is_some_and(|requested| resource_names.iter().any(|name| name == requested))
}

#[cfg(test)]
mod tests {
    use super::{
        is_superuser, parse_service_account_username, rule_matches_request, subject_matches_user,
    };
    use crate::auth::authorization::AuthorizationRequest;
    use crate::auth::user_info::UserInfo;
    use std::collections::HashMap;
    use tugboat_resources::manifests::authorization::v1::{PolicyRule, Subject};

    #[test]
    fn policy_rule_matches_exact_request() {
        let rule = PolicyRule {
            api_groups: vec!["core".to_string()],
            resources: vec!["ships".to_string()],
            verbs: vec!["get".to_string()],
            resource_names: vec![],
        };

        assert!(rule_matches_request(&rule, &request()));
    }

    #[test]
    fn policy_rule_supports_wildcards() {
        let rule = PolicyRule {
            api_groups: vec!["*".to_string()],
            resources: vec!["*".to_string()],
            verbs: vec!["*".to_string()],
            resource_names: vec![],
        };

        assert!(rule_matches_request(&rule, &request()));
    }

    #[test]
    fn policy_rule_requires_matching_resource_name_when_present() {
        let rule = PolicyRule {
            api_groups: vec!["core".to_string()],
            resources: vec!["ships".to_string()],
            verbs: vec!["get".to_string()],
            resource_names: vec!["alpha".to_string()],
        };

        assert!(!rule_matches_request(&rule, &request()));
        assert!(rule_matches_request(
            &rule,
            &AuthorizationRequest {
                resource_name: Some("alpha".to_string()),
                ..request()
            }
        ));
    }

    #[test]
    fn group_subject_matches_group_membership() {
        let subject = Subject {
            kind: "Group".to_string(),
            api_group: "authorization".to_string(),
            name: "developers".to_string(),
            namespace: None,
        };

        assert!(subject_matches_user(&subject, &user()));
    }

    #[test]
    fn service_account_subject_matches_service_account_identity() {
        let subject = Subject {
            kind: "ServiceAccount".to_string(),
            api_group: "".to_string(),
            name: "builder".to_string(),
            namespace: Some("default".to_string()),
        };
        let user = UserInfo::service_account("default", "builder", None, HashMap::new());

        assert!(subject_matches_user(&subject, &user));
    }

    #[test]
    fn parses_service_account_username() {
        assert_eq!(
            parse_service_account_username("system:serviceaccount:default:builder"),
            Some(("default", "builder"))
        );
        assert_eq!(parse_service_account_username("alice"), None);
    }

    #[test]
    fn system_masters_group_is_superuser() {
        let mut user = user();
        user.groups.push("system:masters".to_string());

        assert!(is_superuser(&user));
    }

    fn request() -> AuthorizationRequest {
        AuthorizationRequest {
            user: user(),
            verb: "get".to_string(),
            api_group: "core".to_string(),
            resource: "ships".to_string(),
            resource_name: None,
            namespace: Some("default".to_string()),
        }
    }

    fn user() -> UserInfo {
        UserInfo::x509(
            "alice".to_string(),
            vec!["developers".to_string()],
            HashMap::new(),
        )
    }
}
