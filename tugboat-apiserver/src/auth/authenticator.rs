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
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::SERVICE_ACCOUNT_NAME_ANNOTATION;
use tugboat_resources::manifests::core::v1::{Secret, ServiceAccount};

type AuthResult<'a> =
    Pin<Box<dyn Future<Output = Result<Option<UserInfo>, Box<StatusResponse>>> + 'a>>;

pub(crate) trait Authenticator: Clone + 'static {
    fn authenticate<'a>(&'a self, req: &'a HttpRequest) -> AuthResult<'a>;

    fn anonymous_enabled(&self) -> bool;
}

#[derive(Clone)]
pub(crate) struct DefaultAuthenticator {
    operator: Data<ApiOperator>,
    anonymous_enabled: bool,
}

impl DefaultAuthenticator {
    pub(crate) fn new(operator: Data<ApiOperator>, config: AuthenticationConfig) -> Self {
        Self {
            operator,
            anonymous_enabled: config.anonymous_enabled,
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

    async fn find_service_account_token_secret(
        &self,
        token: &str,
    ) -> Result<Option<Secret>, tugboat_resource_store::error::Error> {
        let secrets = self.operator.store.list::<Secret>(None, None).await?;
        for secret in secrets {
            let secret = secret.apply_revision();
            if !secret_contains_token(&secret, token) {
                continue;
            }
            if service_account_name(&secret).is_some() {
                return Ok(Some(secret));
            }
        }
        Ok(None)
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
    if scheme != "Bearer" || token.is_empty() {
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

fn secret_contains_token(secret: &Secret, token: &str) -> bool {
    secret
        .data
        .get("token")
        .and_then(|encoded| BASE64_STANDARD.decode(encoded).ok())
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .is_some_and(|decoded| decoded == token)
}

#[cfg(test)]
mod tests {
    use super::{
        authenticate_client_certificate, bearer_token, secret_contains_token, service_account_name,
    };
    use crate::auth::middleware::ClientCertificateInfo;
    use actix_web::ResponseError;
    use actix_web::test::TestRequest;
    use std::collections::HashMap;
    use tugboat_resources::manifests::core::v1::Secret;
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn extracts_bearer_token() {
        let req = TestRequest::default()
            .insert_header(("Authorization", "Bearer token-value"))
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
            .insert("token".to_string(), "c2VjcmV0LXRva2Vu".to_string());

        assert!(secret_contains_token(&secret, "secret-token"));
    }

    #[test]
    fn extracts_service_account_name_from_annotations() {
        let secret = Secret {
            object_meta: Some(ObjectMeta {
                annotations: HashMap::from([(
                    "tugboat.io/service-account.name".to_string(),
                    "builder".to_string(),
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(service_account_name(&secret).as_deref(), Some("builder"));
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
