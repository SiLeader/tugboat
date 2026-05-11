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

use crate::auth::middleware::ClientCertificateInfo;
use crate::auth::service_account_jwt::{BoundObjectReference, looks_like_jwt};
use crate::auth::user_info::UserInfo;
use crate::config::AuthenticationConfig;
use crate::data::StatusResponse;
use crate::operator::ApiOperator;
use actix_web::HttpRequest;
use actix_web::http::header::AUTHORIZATION;
use actix_web::web::Data;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tugboat_resource_store::serializer::StaticSerializable;
use tugboat_resources::manifests::apps::v1::{Deployment, Fleet, ReplicaSet};
use tugboat_resources::manifests::coordination::v1::Lease;
use tugboat_resources::manifests::core::v1::{
    ClusterNetworkClass, ConfigMap, Namespace, NetworkClass, Node, PersistentVolume,
    PersistentVolumeClaim, RuntimeClass, Secret, ServiceAccount, Ship, ShipClass, StorageClass,
};
use tugboat_resources::{ObjectMetaResource, StaticResource};
use tugboat_resources::{SERVICE_ACCOUNT_NAME_ANNOTATION, SERVICE_ACCOUNT_TOKEN_SECRET_TYPE};

/// How long the token→secret mapping cache is considered fresh. After this
/// period the cache is rebuilt from a full etcd scan on the next cache miss.
const TOKEN_CACHE_TTL_SECS: u64 = 60;

/// Index that maps a base64-encoded bearer token to the (namespace, name) of
/// the service-account-token Secret that holds it.
///
/// The full-secret scan is O(n) in the number of Secrets; caching allows
/// established service accounts to be authenticated in O(1) via a direct etcd
/// get. The cache is rebuilt from scratch whenever it is stale (TTL expired)
/// or when a token is not found in the current snapshot (to pick up newly
/// created secrets without waiting for the TTL to expire).
#[derive(Default)]
struct TokenCacheState {
    /// base64-encoded token value → (namespace, secret name)
    map: HashMap<String, (String, String)>,
    built_at: Option<Instant>,
}

impl TokenCacheState {
    fn is_stale(&self) -> bool {
        self.built_at
            .map(|t| t.elapsed().as_secs() > TOKEN_CACHE_TTL_SECS)
            .unwrap_or(true)
    }
}

type AuthResult<'a> =
    Pin<Box<dyn Future<Output = Result<Option<UserInfo>, Box<StatusResponse>>> + 'a>>;
const TOKEN_DATA_KEY: &str = "token";

pub(crate) trait Authenticator: Clone + 'static {
    fn authenticate<'a>(&'a self, req: &'a HttpRequest) -> AuthResult<'a>;

    fn anonymous_enabled(&self) -> bool;
}

#[derive(Clone)]
pub(crate) struct DefaultAuthenticator {
    operator: Data<ApiOperator>,
    anonymous_enabled: bool,
    /// Shared cache across all cloned instances of this authenticator.
    token_cache: Arc<RwLock<TokenCacheState>>,
}

impl DefaultAuthenticator {
    pub(crate) fn new(operator: Data<ApiOperator>, config: AuthenticationConfig) -> Self {
        Self {
            operator,
            anonymous_enabled: config.anonymous_enabled,
            token_cache: Arc::new(RwLock::new(TokenCacheState::default())),
        }
    }

    async fn authenticate_impl(
        &self,
        req: &HttpRequest,
    ) -> Result<Option<UserInfo>, Box<StatusResponse>> {
        if let Some(token) = bearer_token(req)? {
            let user = self.authenticate_service_account_token(&token).await?;
            return Ok(Some(user));
        }
        if let Some(cert) = req.conn_data::<ClientCertificateInfo>() {
            let user = authenticate_client_certificate(cert)?;
            return Ok(Some(user));
        }
        Ok(None)
    }

    async fn authenticate_service_account_token(
        &self,
        token: &str,
    ) -> Result<UserInfo, Box<StatusResponse>> {
        if looks_like_jwt(token)
            && let Some(jwt_issuer) = &self.operator.service_account_tokens
            && let Ok(user) = self
                .authenticate_service_account_jwt(jwt_issuer, token)
                .await
        {
            return Ok(user);
        }

        let secret = self
            .find_service_account_token_secret(token)
            .await?
            .ok_or_else(|| Box::new(StatusResponse::unauthorized("Invalid bearer token", None)))?;

        let namespace = secret.namespace().ok_or_else(|| {
            Box::new(StatusResponse::unauthorized(
                "Service account token secret is missing namespace",
                None,
            ))
        })?;
        let service_account_name = service_account_name(&secret).ok_or_else(|| {
            Box::new(StatusResponse::unauthorized(
                "Service account token secret is missing annotation",
                None,
            ))
        })?;
        let service_account = self
            .operator
            .store
            .get::<ServiceAccount>(Some(namespace.to_string()), &service_account_name)
            .await?
            .map(|resource| resource.apply_revision())
            .ok_or_else(|| {
                Box::new(StatusResponse::unauthorized(
                    "Service account for bearer token was not found",
                    None,
                ))
            })?;
        if !service_account_references_secret(&service_account, &secret) {
            return Err(Box::new(StatusResponse::unauthorized(
                "Service account token secret is not referenced by the service account",
                None,
            )));
        }
        let service_account_name = service_account.name().ok_or_else(|| {
            Box::new(StatusResponse::unauthorized(
                "Service account is missing metadata.name",
                None,
            ))
        })?;

        let mut extra = HashMap::new();
        extra.insert(
            "authentication.kubernetes.io/credential".to_string(),
            vec!["Bearer".to_string()],
        );

        Ok(UserInfo::service_account(
            namespace,
            service_account_name,
            service_account
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.uid.clone()),
            extra,
        ))
    }

    async fn authenticate_service_account_jwt(
        &self,
        jwt_issuer: &crate::auth::service_account_jwt::ServiceAccountTokenIssuer,
        token: &str,
    ) -> Result<UserInfo, Box<StatusResponse>> {
        let verified = jwt_issuer
            .verify_token(token)
            .map_err(|_| Box::new(StatusResponse::unauthorized("Invalid bearer token", None)))?;
        let service_account = self
            .operator
            .store
            .get::<ServiceAccount>(Some(verified.namespace.clone()), &verified.name)
            .await?
            .map(|resource| resource.apply_revision())
            .ok_or_else(|| {
                Box::new(StatusResponse::unauthorized(
                    "Service account for bearer token was not found",
                    None,
                ))
            })?;
        let current_uid = service_account
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.uid.clone());
        if verified.uid.is_some() && verified.uid != current_uid {
            return Err(Box::new(StatusResponse::unauthorized(
                "Service account token UID does not match current service account",
                None,
            )));
        }
        if let Some(bound_object_ref) = &verified.bound_object_ref {
            self.validate_bound_object_ref(&verified.namespace, bound_object_ref)
                .await?;
        }

        let mut extra = HashMap::new();
        extra.insert(
            "authentication.kubernetes.io/credential".to_string(),
            vec!["Bearer/JWT".to_string()],
        );

        Ok(UserInfo::service_account(
            &verified.namespace,
            &verified.name,
            current_uid,
            extra,
        ))
    }

    async fn validate_bound_object_ref(
        &self,
        service_account_namespace: &str,
        reference: &BoundObjectReference,
    ) -> Result<(), Box<StatusResponse>> {
        match (reference.api_version.as_str(), reference.kind.as_str()) {
            ("v1", "ClusterNetworkClass") => {
                self.validate_typed_bound_object::<ClusterNetworkClass>(None, reference)
                    .await
            }
            ("v1", "ConfigMap") => {
                self.validate_typed_bound_object::<ConfigMap>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            ("v1", "Namespace") => {
                self.validate_typed_bound_object::<Namespace>(None, reference)
                    .await
            }
            ("v1", "NetworkClass") => {
                self.validate_typed_bound_object::<NetworkClass>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            ("v1", "Node") => {
                self.validate_typed_bound_object::<Node>(None, reference)
                    .await
            }
            ("v1", "PersistentVolume") => {
                self.validate_typed_bound_object::<PersistentVolume>(None, reference)
                    .await
            }
            ("v1", "PersistentVolumeClaim") => {
                self.validate_typed_bound_object::<PersistentVolumeClaim>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            ("v1", "RuntimeClass") => {
                self.validate_typed_bound_object::<RuntimeClass>(None, reference)
                    .await
            }
            ("v1", "Secret") => {
                self.validate_typed_bound_object::<Secret>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            ("v1", "ServiceAccount") => {
                self.validate_typed_bound_object::<ServiceAccount>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            ("v1", "Ship") => {
                self.validate_typed_bound_object::<Ship>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            ("v1", "ShipClass") => {
                self.validate_typed_bound_object::<ShipClass>(None, reference)
                    .await
            }
            ("v1", "StorageClass") => {
                self.validate_typed_bound_object::<StorageClass>(None, reference)
                    .await
            }
            ("apps/v1", "Deployment") => {
                self.validate_typed_bound_object::<Deployment>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            ("apps/v1", "Fleet") => {
                self.validate_typed_bound_object::<Fleet>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            ("apps/v1", "ReplicaSet") => {
                self.validate_typed_bound_object::<ReplicaSet>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            ("coordination/v1", "Lease") => {
                self.validate_typed_bound_object::<Lease>(
                    Some(service_account_namespace.to_string()),
                    reference,
                )
                .await
            }
            _ => Err(Box::new(StatusResponse::unauthorized(
                "Service account token bound object kind is not supported",
                None,
            ))),
        }
    }

    async fn validate_typed_bound_object<T>(
        &self,
        namespace: Option<String>,
        reference: &BoundObjectReference,
    ) -> Result<(), Box<StatusResponse>>
    where
        T: StaticSerializable + StaticResource + ObjectMetaResource,
    {
        let expected_namespace = if T::is_cluster_scoped() {
            None
        } else {
            namespace
        };
        let Some(object) = self
            .operator
            .store
            .get::<T>(expected_namespace, &reference.name)
            .await?
            .map(|resource| resource.apply_revision())
        else {
            return Err(Box::new(StatusResponse::unauthorized(
                "Service account token bound object was not found",
                None,
            )));
        };
        if let Some(expected_uid) = &reference.uid {
            let actual_uid = object
                .object_meta()
                .as_ref()
                .and_then(|meta| meta.uid.as_ref());
            if actual_uid != Some(expected_uid) {
                return Err(Box::new(StatusResponse::unauthorized(
                    "Service account token bound object UID does not match current object",
                    None,
                )));
            }
        }
        Ok(())
    }

    async fn find_service_account_token_secret(
        &self,
        token: &str,
    ) -> Result<Option<Secret>, tugboat_resource_store::error::Error> {
        let encoded = BASE64_STANDARD.encode(token);

        // --- Fast path: cache hit ---
        // The cache stores base64-encoded token → (namespace, secret name).
        // On a hit, perform a single direct etcd GET instead of a full scan.
        {
            let cache = self.token_cache.read().await;
            if !cache.is_stale()
                && let Some((namespace, name)) = cache.map.get(&encoded)
            {
                // Direct O(1) lookup; verify the secret still exists and
                // still contains the expected token (handles deletion / rotation).
                if let Some(data) = self
                    .operator
                    .store
                    .get::<Secret>(Some(namespace.clone()), name)
                    .await?
                {
                    let secret = data.apply_revision();
                    if secret_contains_token(&secret, token) {
                        return Ok(Some(secret));
                    }
                }
                // Secret was deleted or token rotated — fall through to rebuild.
            }
            // Token not in current cache snapshot → fall through to rebuild so
            // newly created service-account tokens are picked up immediately.
        }

        // --- Slow path: full scan ---
        // Rebuild the cache from all secrets currently in etcd and locate the
        // matching secret while we have the data in hand.
        let secrets = self.operator.store.list::<Secret>(None, None).await?;
        let mut new_map = HashMap::new();
        let mut found: Option<Secret> = None;

        for secret_data in secrets {
            let secret = secret_data.apply_revision();
            if !is_service_account_token_secret(&secret) {
                continue;
            }
            if service_account_name(&secret).is_none() {
                continue;
            }
            let (Some(ns), Some(name)) = (secret.namespace(), secret.name()) else {
                continue;
            };
            if let Some(stored_encoded) = secret.data.get(TOKEN_DATA_KEY) {
                new_map.insert(stored_encoded.clone(), (ns.to_string(), name.to_string()));
                if stored_encoded == &encoded && found.is_none() {
                    found = Some(secret);
                }
            }
        }

        // Replace the entire cache with the fresh snapshot so that deleted
        // secrets are automatically evicted.
        {
            let mut cache = self.token_cache.write().await;
            cache.map = new_map;
            cache.built_at = Some(Instant::now());
        }

        Ok(found)
    }
}

impl Authenticator for DefaultAuthenticator {
    fn authenticate<'a>(&'a self, req: &'a HttpRequest) -> AuthResult<'a> {
        Box::pin(self.authenticate_impl(req))
    }

    fn anonymous_enabled(&self) -> bool {
        self.anonymous_enabled
    }
}

fn bearer_token(req: &HttpRequest) -> Result<Option<String>, Box<StatusResponse>> {
    let Some(value) = req.headers().get(AUTHORIZATION) else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| {
        Box::new(StatusResponse::unauthorized(
            "Authorization header is not valid ASCII",
            None,
        ))
    })?;
    let Some((scheme, token)) = value.split_once(' ') else {
        return Err(Box::new(StatusResponse::unauthorized(
            "Authorization header must use the format 'Bearer <token>'",
            None,
        )));
    };
    let token = token.trim();
    if !scheme.eq_ignore_ascii_case("Bearer") || token.is_empty() {
        return Err(Box::new(StatusResponse::unauthorized(
            "Authorization header must use the Bearer scheme",
            None,
        )));
    }
    Ok(Some(token.to_string()))
}

fn authenticate_client_certificate(
    cert: &ClientCertificateInfo,
) -> Result<UserInfo, Box<StatusResponse>> {
    let username = cert.common_name.clone().ok_or_else(|| {
        Box::new(StatusResponse::unauthorized(
            "Client certificate is missing Common Name",
            None,
        ))
    })?;
    let mut extra = HashMap::new();
    extra.insert(
        "authentication.kubernetes.io/credential".to_string(),
        vec!["X509".to_string()],
    );
    Ok(UserInfo::x509(username, cert.organizations.clone(), extra))
}

fn service_account_name(secret: &Secret) -> Option<String> {
    secret
        .object_meta
        .as_ref()?
        .annotations
        .get(SERVICE_ACCOUNT_NAME_ANNOTATION)
        .cloned()
}

fn is_service_account_token_secret(secret: &Secret) -> bool {
    secret.r#type == SERVICE_ACCOUNT_TOKEN_SECRET_TYPE
}

fn service_account_references_secret(service_account: &ServiceAccount, secret: &Secret) -> bool {
    let Some(secret_name) = secret.name() else {
        return false;
    };
    let Some(secret_namespace) = secret.namespace() else {
        return false;
    };
    let secret_uid = secret
        .object_meta()
        .as_ref()
        .and_then(|meta| meta.uid.as_deref());

    service_account.secrets.iter().any(|reference| {
        reference.kind == "Secret"
            && reference.api_version == "v1"
            && reference.name == secret_name
            && reference.namespace.as_deref() == Some(secret_namespace)
            && match secret_uid {
                Some(secret_uid) => reference.uid == secret_uid,
                None => false,
            }
    })
}

fn secret_contains_token(secret: &Secret, token: &str) -> bool {
    secret
        .data
        .get(TOKEN_DATA_KEY)
        .and_then(|encoded| BASE64_STANDARD.decode(encoded).ok())
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .is_some_and(|decoded| decoded == token)
}

#[cfg(test)]
mod tests {
    use super::{
        TOKEN_DATA_KEY, authenticate_client_certificate, bearer_token,
        is_service_account_token_secret, secret_contains_token, service_account_name,
        service_account_references_secret,
    };
    use crate::auth::middleware::ClientCertificateInfo;
    use actix_web::ResponseError;
    use actix_web::test::TestRequest;
    use std::collections::HashMap;
    use tugboat_resources::SERVICE_ACCOUNT_TOKEN_SECRET_TYPE;
    use tugboat_resources::manifests::core::v1::{Secret, ServiceAccount};
    use tugboat_resources::manifests::meta::v1::{ObjectMeta, ObjectReference};

    #[test]
    fn extracts_bearer_token() {
        let req = TestRequest::default()
            .insert_header(("Authorization", "Bearer token-value"))
            .to_http_request();

        let token = bearer_token(&req).expect("header should parse");

        assert_eq!(token.as_deref(), Some("token-value"));
    }

    #[test]
    fn extracts_bearer_token_case_insensitive() {
        let req = TestRequest::default()
            .insert_header(("Authorization", "bearer token-value"))
            .to_http_request();

        let token = bearer_token(&req).expect("header should parse");

        assert_eq!(token.as_deref(), Some("token-value"));
    }

    #[test]
    fn rejects_non_bearer_authorization() {
        let req = TestRequest::default()
            .insert_header(("Authorization", "Basic abc"))
            .to_http_request();

        let err = bearer_token(&req).expect_err("header should be rejected");

        assert_eq!(err.status_code(), actix_web::http::StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn token_secret_match_decodes_base64() {
        let mut secret = Secret::default();
        secret
            .data
            .insert(TOKEN_DATA_KEY.to_string(), "c2VjcmV0LXRva2Vu".to_string());

        assert!(secret_contains_token(&secret, "secret-token"));
    }

    #[test]
    fn extracts_service_account_name_from_annotations() {
        let secret = Secret {
            object_meta: Some(ObjectMeta {
                annotations: HashMap::from([(
                    "tugboat.cloud/service-account.name".to_string(),
                    "builder".to_string(),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(service_account_name(&secret).as_deref(), Some("builder"));
    }

    #[test]
    fn token_secret_requires_service_account_token_type() {
        let secret = Secret {
            r#type: SERVICE_ACCOUNT_TOKEN_SECRET_TYPE.to_string(),
            ..Default::default()
        };

        assert!(is_service_account_token_secret(&secret));
        assert!(!is_service_account_token_secret(&Secret::default()));
    }

    #[test]
    fn service_account_reference_must_match_secret_identity() {
        let secret = Secret {
            object_meta: Some(ObjectMeta {
                name: Some("builder-token".to_string()),
                namespace: Some("default".to_string()),
                uid: Some("secret-uid".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let service_account = ServiceAccount {
            secrets: vec![ObjectReference {
                kind: "Secret".to_string(),
                namespace: Some("default".to_string()),
                name: "builder-token".to_string(),
                uid: "secret-uid".to_string(),
                api_version: "v1".to_string(),
            }],
            ..Default::default()
        };

        assert!(service_account_references_secret(&service_account, &secret));

        let mut secret_without_uid = secret.clone();
        secret_without_uid
            .object_meta
            .as_mut()
            .expect("object_meta should be set")
            .uid = None;
        assert!(!service_account_references_secret(
            &service_account,
            &secret_without_uid
        ));

        let mut missing_uid = service_account.clone();
        missing_uid.secrets[0].uid.clear();
        assert!(!service_account_references_secret(&missing_uid, &secret));

        let mut wrong_version = service_account;
        wrong_version.secrets[0].api_version = "v1beta1".to_string();
        assert!(!service_account_references_secret(&wrong_version, &secret));
    }

    #[test]
    fn certificate_authentication_requires_common_name() {
        let err = authenticate_client_certificate(&ClientCertificateInfo {
            common_name: None,
            organizations: vec!["ops".to_string()],
        })
        .expect_err("certificate without CN should fail");

        assert_eq!(err.status_code(), actix_web::http::StatusCode::UNAUTHORIZED);
    }
}
