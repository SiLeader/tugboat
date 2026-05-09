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

use crate::auth::authenticator::{Authenticator, DefaultAuthenticator};
use crate::auth::authorization::{AuthorizationDecision, AuthorizationRequest};
use crate::auth::rbac_authorizer::RbacAuthorizer;
use crate::auth::user_info::UserInfo;
use crate::config::{AuthenticationConfig, AuthorizationConfig, AuthorizationMode};
use crate::data::StatusResponse;
use crate::endpoints::resource_registry;
use crate::operator::ApiOperator;
use actix_web::Error;
use actix_web::body::EitherBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform, forward_ready};
use actix_web::http::Method;
use actix_web::web::Data;
use actix_web::{HttpMessage, HttpRequest, ResponseError};
use openssl::nid::Nid;
use openssl::x509::X509Ref;
use std::future::{Future, Ready, ready};
use std::pin::Pin;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClientCertificateInfo {
    pub(crate) common_name: Option<String>,
    pub(crate) organizations: Vec<String>,
}

impl ClientCertificateInfo {
    pub(crate) fn from_x509(cert: &X509Ref) -> Self {
        Self {
            common_name: first_entry_by_nid(cert, Nid::COMMONNAME),
            organizations: entries_by_nid(cert, Nid::ORGANIZATIONNAME),
        }
    }
}

fn first_entry_by_nid(cert: &X509Ref, nid: Nid) -> Option<String> {
    cert.subject_name()
        .entries_by_nid(nid)
        .next()
        .and_then(|entry| entry.data().as_utf8().ok().map(|value| value.to_string()))
}

fn entries_by_nid(cert: &X509Ref, nid: Nid) -> Vec<String> {
    cert.subject_name()
        .entries_by_nid(nid)
        .filter_map(|entry| entry.data().as_utf8().ok().map(|value| value.to_string()))
        .collect()
}

#[derive(Clone)]
pub(crate) struct AuthenticationMiddleware {
    authenticator: DefaultAuthenticator,
}

#[derive(Clone)]
pub(crate) struct AuthorizationMiddleware {
    operator: Data<ApiOperator>,
    config: AuthorizationConfig,
}

impl AuthenticationMiddleware {
    pub(crate) fn new(operator: Data<ApiOperator>, config: AuthenticationConfig) -> Self {
        Self {
            authenticator: DefaultAuthenticator::new(operator, config),
        }
    }
}

impl AuthorizationMiddleware {
    pub(crate) fn new(operator: Data<ApiOperator>, config: AuthorizationConfig) -> Self {
        Self { operator, config }
    }
}

impl<S, B> Transform<S, ServiceRequest> for AuthenticationMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthenticationMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuthenticationMiddlewareService {
            service: Rc::new(service),
            authenticator: self.authenticator.clone(),
        }))
    }
}

pub(crate) struct AuthenticationMiddlewareService<S> {
    service: Rc<S>,
    authenticator: DefaultAuthenticator,
}

pub(crate) struct AuthorizationMiddlewareService<S> {
    service: Rc<S>,
    operator: Data<ApiOperator>,
    config: AuthorizationConfig,
}

impl<S, B> Service<ServiceRequest> for AuthenticationMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let authenticator = self.authenticator.clone();
        let service = Rc::clone(&self.service);

        Box::pin(async move {
            if should_bypass_authentication(req.path()) {
                req.extensions_mut().insert(UserInfo::anonymous());
                let res = service.call(req).await?;
                return Ok(res.map_into_left_body());
            }
            let user = match authenticator.authenticate(req.request()).await {
                Ok(Some(user)) => user,
                Ok(None) if authenticator.anonymous_enabled() => UserInfo::anonymous(),
                Ok(None) => {
                    return unauthorized(req, "Authentication credentials were not provided");
                }
                Err(err) => {
                    return unauthorized(req, err.to_string());
                }
            };

            req.extensions_mut().insert(user);
            let res = service.call(req).await?;
            Ok(res.map_into_left_body())
        })
    }
}

impl<S, B> Transform<S, ServiceRequest> for AuthorizationMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthorizationMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuthorizationMiddlewareService {
            service: Rc::new(service),
            operator: self.operator.clone(),
            config: self.config.clone(),
        }))
    }
}

impl<S, B> Service<ServiceRequest> for AuthorizationMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = Rc::clone(&self.service);
        let operator = self.operator.clone();
        let config = self.config.clone();

        Box::pin(async move {
            if matches!(config.mode, AuthorizationMode::AlwaysAllow) || should_bypass(req.path()) {
                let res = service.call(req).await?;
                return Ok(res.map_into_left_body());
            }

            let Some(user) = req.extensions().get::<UserInfo>().cloned() else {
                return unauthorized(req, "Authentication context is missing");
            };

            let Some(authz_request) = build_authorization_request(req.request(), user) else {
                let res = service.call(req).await?;
                return Ok(res.map_into_left_body());
            };

            match RbacAuthorizer::authorize_with_store(&operator.store, &authz_request).await {
                AuthorizationDecision::Allowed => {
                    let res = service.call(req).await?;
                    Ok(res.map_into_left_body())
                }
                AuthorizationDecision::Denied { reason } => forbidden(req, reason),
            }
        })
    }
}

fn unauthorized<B>(
    req: ServiceRequest,
    message: impl ToString,
) -> Result<ServiceResponse<EitherBody<B>>, Error> {
    let response = StatusResponse::unauthorized(message, None).error_response();
    Ok(req.into_response(response).map_into_right_body())
}

fn forbidden<B>(
    req: ServiceRequest,
    message: impl ToString,
) -> Result<ServiceResponse<EitherBody<B>>, Error> {
    let response = StatusResponse::forbidden(message, None).error_response();
    Ok(req.into_response(response).map_into_right_body())
}

fn should_bypass_authentication(path: &str) -> bool {
    path == "/healthz"
}

fn should_bypass(path: &str) -> bool {
    matches!(
        path,
        "/healthz" | "/apis" | "/api" | "/api/v1" | "/openapi.json"
    ) || path == "/openapi/v3"
        || path.starts_with("/openapi/v3/")
}

fn build_authorization_request(req: &HttpRequest, user: UserInfo) -> Option<AuthorizationRequest> {
    let path = req.path();
    let mut segments = path
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty());

    let (api_group, version) = match segments.next()? {
        "api" => ("core".to_string(), segments.next()?.to_string()),
        "apis" => (segments.next()?.to_string(), segments.next()?.to_string()),
        _ => return None,
    };

    let remaining = segments.collect::<Vec<_>>();
    let parsed = parse_resource_path(api_group.as_str(), version.as_str(), &remaining)?;
    let verb = request_verb(
        req.method(),
        req.query_string(),
        parsed.resource_name.is_some(),
    )?;

    Some(AuthorizationRequest {
        user,
        verb,
        api_group,
        resource: parsed.resource,
        resource_name: parsed.resource_name,
        namespace: parsed.namespace,
    })
}

struct ParsedResourcePath {
    resource: String,
    resource_name: Option<String>,
    namespace: Option<String>,
}

fn parse_resource_path(
    api_group: &str,
    version: &str,
    path: &[&str],
) -> Option<ParsedResourcePath> {
    let descriptors = resource_registry::resources_for(api_group, version);
    let (namespace, resource_index) = match path {
        ["namespaces", namespace, resource, ..] => {
            match descriptors.iter().find(|entry| entry.plural == *resource) {
                Some(descriptor) => (descriptor.namespaced().then(|| (*namespace).to_string()), 2),
                None => (None, 0),
            }
        }
        _ => (None, 0),
    };

    let resource = *path.get(resource_index)?;
    let descriptor = descriptors
        .into_iter()
        .find(|entry| entry.plural == resource)?;

    let path_namespace = if descriptor.namespaced() {
        namespace
    } else {
        None
    };
    let after_resource = &path[(resource_index + 1)..];
    let resource_name = after_resource.first().map(|segment| (*segment).to_string());
    let resource = if after_resource.len() > 1 {
        format!("{resource}/{}", after_resource[1..].join("/"))
    } else {
        resource.to_string()
    };

    Some(ParsedResourcePath {
        resource,
        resource_name,
        namespace: path_namespace,
    })
}

fn request_verb(method: &Method, query: &str, resource_name_present: bool) -> Option<String> {
    let verb = match *method {
        Method::GET if query_requests_watch(query) => "watch",
        Method::GET if resource_name_present => "get",
        Method::GET => "list",
        Method::POST => "create",
        Method::PUT => "update",
        Method::PATCH => "patch",
        Method::DELETE => "delete",
        // HEAD, OPTIONS, and other methods return None, which causes
        // build_authorization_request to return None and the request to pass
        // through without an authorization check. This is intentional: these
        // methods are used for discovery and CORS preflight and do not access
        // or mutate resource data.
        _ => return None,
    };
    Some(verb.to_string())
}

fn query_requests_watch(query: &str) -> bool {
    query.split('&').any(|pair| {
        let Some((key, value)) = pair.split_once('=') else {
            return false;
        };
        key == "watch" && matches!(value, "true" | "True" | "ndJson" | "NdJson")
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ClientCertificateInfo, build_authorization_request, parse_resource_path,
        query_requests_watch,
    };
    use crate::auth::user_info::UserInfo;
    use crate::endpoints::resource_registry;
    use actix_web::test::TestRequest;
    use openssl::asn1::Asn1Time;
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::x509::{X509, X509NameBuilder};

    #[test]
    fn extracts_identity_from_certificate_subject() {
        let cert = test_cert(&[("CN", "client-user"), ("O", "ops"), ("O", "sre")]);

        let info = ClientCertificateInfo::from_x509(cert.as_ref());

        assert_eq!(info.common_name.as_deref(), Some("client-user"));
        assert_eq!(
            info.organizations,
            vec!["ops".to_string(), "sre".to_string()]
        );
    }

    #[test]
    fn missing_subject_entries_are_empty() {
        let cert = test_cert(&[("O", "ops")]);

        let info = ClientCertificateInfo::from_x509(cert.as_ref());

        assert_eq!(info.common_name, None);
        assert_eq!(info.organizations, vec!["ops".to_string()]);
    }

    #[test]
    fn builds_authorization_request_for_namespaced_read() {
        let req = TestRequest::get()
            .uri("/api/v1/namespaces/default/configmaps/example")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous()).unwrap();

        assert_eq!(request.api_group, "core");
        assert_eq!(request.verb, "get");
        assert_eq!(request.resource, "configmaps");
        assert_eq!(request.resource_name.as_deref(), Some("example"));
        assert_eq!(request.namespace.as_deref(), Some("default"));
    }

    #[test]
    fn builds_authorization_request_for_status_subresource() {
        let req = TestRequest::patch()
            .uri("/apis/apps/v1/namespaces/default/deployments/example/status")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous()).unwrap();

        assert_eq!(request.api_group, "apps");
        assert_eq!(request.verb, "patch");
        assert_eq!(request.resource, "deployments/status");
        assert_eq!(request.resource_name.as_deref(), Some("example"));
        assert_eq!(request.namespace.as_deref(), Some("default"));
    }

    #[test]
    fn builds_authorization_request_for_custom_action_path() {
        let req = TestRequest::post()
            .uri("/api/v1/namespaces/default/ships/example/migrate/abort")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous()).unwrap();

        assert_eq!(request.verb, "create");
        assert_eq!(request.resource, "ships/migrate/abort");
        assert_eq!(request.resource_name.as_deref(), Some("example"));
    }

    #[test]
    fn cluster_scoped_resource_ignores_namespace_prefix() {
        let parsed = parse_resource_path("core", "v1", &["namespaces", "default", "namespaces"])
            .expect("resource path should parse");

        assert_eq!(parsed.resource, "namespaces");
        assert_eq!(parsed.namespace, None);
        assert_eq!(parsed.resource_name, None);
    }

    #[test]
    fn parses_cluster_scoped_namespace_collection() {
        let req = TestRequest::get()
            .uri("/api/v1/namespaces")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous()).unwrap();

        assert_eq!(request.verb, "list");
        assert_eq!(request.resource, "namespaces");
        assert_eq!(request.resource_name, None);
        assert_eq!(request.namespace, None);
    }

    #[test]
    fn parses_cluster_scoped_namespace_read() {
        let req = TestRequest::get()
            .uri("/api/v1/namespaces/default")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous()).unwrap();

        assert_eq!(request.verb, "get");
        assert_eq!(request.resource, "namespaces");
        assert_eq!(request.resource_name.as_deref(), Some("default"));
        assert_eq!(request.namespace, None);
    }

    #[test]
    fn recognizes_watch_queries() {
        assert!(query_requests_watch("watch=true"));
        assert!(query_requests_watch("resourceVersion=10&watch=ndJson"));
        assert!(!query_requests_watch("watch=false"));
    }

    #[test]
    fn rbac_resource_names_are_derived_from_resource_descriptors() {
        for descriptor in resource_registry::all_resource_apis() {
            let collection_path = if descriptor.namespaced() {
                vec!["namespaces", "default", descriptor.plural]
            } else {
                vec![descriptor.plural]
            };
            let parsed =
                parse_resource_path(descriptor.group, descriptor.version, &collection_path)
                    .expect("collection path should parse");
            assert_eq!(parsed.resource, descriptor.plural);
            assert_eq!(
                parsed.namespace.as_deref(),
                descriptor.namespaced().then_some("default")
            );

            if descriptor.operations.has_status_subresource() {
                let status_path = if descriptor.namespaced() {
                    vec![
                        "namespaces",
                        "default",
                        descriptor.plural,
                        "example",
                        "status",
                    ]
                } else {
                    vec![descriptor.plural, "example", "status"]
                };
                let parsed =
                    parse_resource_path(descriptor.group, descriptor.version, &status_path)
                        .expect("status path should parse");
                assert_eq!(parsed.resource, format!("{}/status", descriptor.plural));
                assert_eq!(parsed.resource_name.as_deref(), Some("example"));
                assert_eq!(
                    parsed.namespace.as_deref(),
                    descriptor.namespaced().then_some("default")
                );
            }
        }
    }

    fn test_cert(subject_entries: &[(&str, &str)]) -> X509 {
        let rsa = Rsa::generate(2048).expect("rsa should be generated");
        let pkey = PKey::from_rsa(rsa).expect("pkey should be generated");
        let mut name = X509NameBuilder::new().expect("name builder should be created");
        for (key, value) in subject_entries {
            name.append_entry_by_text(key, value)
                .expect("subject field should be set");
        }
        let name = name.build();

        let mut cert = X509::builder().expect("cert builder should be created");
        cert.set_version(2).expect("version should be set");
        cert.set_subject_name(&name)
            .expect("subject name should be set");
        cert.set_issuer_name(&name)
            .expect("issuer name should be set");
        cert.set_pubkey(&pkey).expect("public key should be set");
        let not_before = Asn1Time::days_from_now(0).expect("not_before should be created");
        let not_after = Asn1Time::days_from_now(1).expect("not_after should be created");
        cert.set_not_before(&not_before)
            .expect("not_before should be set");
        cert.set_not_after(&not_after)
            .expect("not_after should be set");
        cert.sign(&pkey, MessageDigest::sha256())
            .expect("certificate should be signed");
        cert.build()
    }
}
