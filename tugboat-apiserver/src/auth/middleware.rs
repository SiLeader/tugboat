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

use crate::auth::audit::RecordedAuthorizationDecision;
use crate::auth::authenticator::{Authenticator, DefaultAuthenticator};
use crate::auth::authorization::{AuthorizationDecision, AuthorizationRequest};
use crate::auth::rbac_authorizer::RbacAuthorizer;
use crate::auth::user_info::UserInfo;
use crate::config::{AuthenticationConfig, AuthorizationConfig, AuthorizationMode};
use crate::crd_registry::{CrdRegistry, CrdScope};
use crate::data::StatusResponse;
use crate::endpoints::resource_registry;
use crate::operator::ApiOperator;
use actix_web::Error;
use actix_web::body::EitherBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform, forward_ready};
use actix_web::http::Method;
use actix_web::web::Data;
use actix_web::{HttpMessage, HttpRequest, ResponseError};
use std::future::{Future, Ready, ready};
use std::pin::Pin;
use std::rc::Rc;
use x509_parser::asn1_rs::{Any, BmpString, Tag, UniversalString};
use x509_parser::parse_x509_certificate;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClientCertificateInfo {
    pub(crate) common_name: Option<String>,
    pub(crate) organizations: Vec<String>,
}

impl ClientCertificateInfo {
    pub(crate) fn from_der(der: &[u8]) -> Result<Self, String> {
        let (remaining, cert) = parse_x509_certificate(der)
            .map_err(|err| format!("invalid X.509 certificate: {err}"))?;
        if !remaining.is_empty() {
            return Err("invalid X.509 certificate: trailing data".to_string());
        }
        let common_name = cert
            .subject()
            .iter_common_name()
            .next()
            .map(|entry| subject_value_to_string(entry.attr_value(), "Common Name"))
            .transpose()?;
        let organizations = cert
            .subject()
            .iter_organization()
            .map(|entry| subject_value_to_string(entry.attr_value(), "Organization"))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            common_name,
            organizations,
        })
    }
}

fn subject_value_to_string(value: &Any<'_>, name: &str) -> Result<String, String> {
    match value.tag() {
        Tag::BmpString => BmpString::try_from(value)
            .map(|value| value.string())
            .map_err(|err| format!("invalid X.509 {name} BMPString: {err}")),
        Tag::UniversalString => UniversalString::try_from(value)
            .map(|value| value.string())
            .map_err(|err| format!("invalid X.509 {name} UniversalString: {err}")),
        Tag::NumericString
        | Tag::PrintableString
        | Tag::TeletexString
        | Tag::VisibleString
        | Tag::GeneralString
        | Tag::ObjectDescriptor
        | Tag::GraphicString
        | Tag::VideotexString
        | Tag::Utf8String
        | Tag::Ia5String => std::str::from_utf8(value.as_bytes())
            .map(str::to_owned)
            .map_err(|err| format!("invalid X.509 {name} string: {err}")),
        tag => Err(format!("unsupported X.509 {name} string type: {tag:?}")),
    }
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

            let Some(authz_request) =
                build_authorization_request(req.request(), user, &operator.crd_registry)
            else {
                let res = service.call(req).await?;
                return Ok(res.map_into_left_body());
            };

            let decision =
                RbacAuthorizer::authorize_with_store(&operator.store, &authz_request).await;
            req.extensions_mut()
                .insert(RecordedAuthorizationDecision(decision.clone()));
            match decision {
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
    matches!(
        path,
        "/healthz" | "/openid/v1/jwks" | "/.well-known/openid-configuration"
    )
}

fn should_bypass(path: &str) -> bool {
    matches!(
        path,
        "/healthz"
            | "/apis"
            | "/api"
            | "/api/v1"
            | "/openapi.json"
            | "/openid/v1/jwks"
            | "/.well-known/openid-configuration"
    ) || path == "/openapi/v3"
        || path.starts_with("/openapi/v3/")
}

fn build_authorization_request(
    req: &HttpRequest,
    user: UserInfo,
    crd_registry: &CrdRegistry,
) -> Option<AuthorizationRequest> {
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
    let parsed = parse_resource_path(
        api_group.as_str(),
        version.as_str(),
        &remaining,
        crd_registry,
    )?;
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
    crd_registry: &CrdRegistry,
) -> Option<ParsedResourcePath> {
    parse_static_resource_path(api_group, version, path)
        .or_else(|| parse_crd_resource_path(api_group, version, path, crd_registry))
}

fn parse_static_resource_path(
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

fn parse_crd_resource_path(
    api_group: &str,
    version: &str,
    path: &[&str],
    crd_registry: &CrdRegistry,
) -> Option<ParsedResourcePath> {
    let (entry, namespace, resource_index) = match path {
        ["namespaces", namespace, resource, ..] => {
            let entry = crd_registry.lookup(api_group, version, resource)?;
            let namespace =
                matches!(entry.scope, CrdScope::Namespaced).then(|| (*namespace).to_string());
            (entry, namespace, 2)
        }
        [resource, ..] => (crd_registry.lookup(api_group, version, resource)?, None, 0),
        _ => return None,
    };

    let after_resource = &path[(resource_index + 1)..];
    let resource_name = after_resource.first().map(|segment| (*segment).to_string());
    let resource = if after_resource.len() > 1 {
        format!("{}/{}", entry.plural, after_resource[1..].join("/"))
    } else {
        entry.plural
    };

    Some(ParsedResourcePath {
        resource,
        resource_name,
        namespace,
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
        query_requests_watch, subject_value_to_string,
    };
    use crate::auth::user_info::UserInfo;
    use crate::crd_registry::{CrdEntry, CrdRegistry, CrdScope, CrdVersionInfo};
    use crate::endpoints::resource_registry;
    use actix_web::test::TestRequest;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
    use x509_parser::asn1_rs::{Any, Tag};

    #[test]
    fn extracts_identity_from_certificate_subject() {
        let cert = STANDARD
            .decode(MULTIPLE_ORGANIZATIONS_CERT_DER_BASE64)
            .expect("certificate fixture should decode");

        let info = ClientCertificateInfo::from_der(&cert).expect("certificate should parse");

        assert_eq!(info.common_name.as_deref(), Some("client-user"));
        assert_eq!(
            info.organizations,
            vec!["ops".to_string(), "sre".to_string()]
        );
    }

    #[test]
    fn missing_subject_entries_are_empty() {
        let cert = test_cert(&[("O", "ops")]);

        let info = ClientCertificateInfo::from_der(&cert).expect("certificate should parse");

        assert_eq!(info.common_name, None);
        assert_eq!(info.organizations, vec!["ops".to_string()]);
    }

    #[test]
    fn rejects_malformed_certificate_der() {
        assert!(ClientCertificateInfo::from_der(&[0, 1, 2]).is_err());
    }

    #[test]
    fn decodes_non_utf8_directory_string_encodings() {
        let bmp = Any::from_tag_and_data(Tag::BmpString, &[0, b'u', 0, b's', 0, b'e', 0, b'r']);
        let universal = Any::from_tag_and_data(
            Tag::UniversalString,
            &[0, 0, 0, b'u', 0, 0, 0, b's', 0, 0, 0, b'e', 0, 0, 0, b'r'],
        );
        let teletex = Any::from_tag_and_data(Tag::TeletexString, b"user");

        assert_eq!(
            subject_value_to_string(&bmp, "Common Name").expect("BMPString should parse"),
            "user"
        );
        assert_eq!(
            subject_value_to_string(&universal, "Common Name")
                .expect("UniversalString should parse"),
            "user"
        );
        assert_eq!(
            subject_value_to_string(&teletex, "Common Name").expect("TeletexString should parse"),
            "user"
        );
    }

    #[test]
    fn builds_authorization_request_for_namespaced_read() {
        let registry = CrdRegistry::default();
        let req = TestRequest::get()
            .uri("/api/v1/namespaces/default/configmaps/example")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous(), &registry).unwrap();

        assert_eq!(request.api_group, "core");
        assert_eq!(request.verb, "get");
        assert_eq!(request.resource, "configmaps");
        assert_eq!(request.resource_name.as_deref(), Some("example"));
        assert_eq!(request.namespace.as_deref(), Some("default"));
    }

    #[test]
    fn builds_authorization_request_for_status_subresource() {
        let registry = CrdRegistry::default();
        let req = TestRequest::patch()
            .uri("/apis/apps/v1/namespaces/default/deployments/example/status")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous(), &registry).unwrap();

        assert_eq!(request.api_group, "apps");
        assert_eq!(request.verb, "patch");
        assert_eq!(request.resource, "deployments/status");
        assert_eq!(request.resource_name.as_deref(), Some("example"));
        assert_eq!(request.namespace.as_deref(), Some("default"));
    }

    #[test]
    fn builds_authorization_request_for_custom_action_path() {
        let registry = CrdRegistry::default();
        let req = TestRequest::post()
            .uri("/api/v1/namespaces/default/ships/example/migrate/abort")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous(), &registry).unwrap();

        assert_eq!(request.verb, "create");
        assert_eq!(request.resource, "ships/migrate/abort");
        assert_eq!(request.resource_name.as_deref(), Some("example"));
    }

    #[test]
    fn cluster_scoped_resource_ignores_namespace_prefix() {
        let registry = CrdRegistry::default();
        let parsed = parse_resource_path(
            "core",
            "v1",
            &["namespaces", "default", "namespaces"],
            &registry,
        )
        .expect("resource path should parse");

        assert_eq!(parsed.resource, "namespaces");
        assert_eq!(parsed.namespace, None);
        assert_eq!(parsed.resource_name, None);
    }

    #[test]
    fn parses_cluster_scoped_namespace_collection() {
        let registry = CrdRegistry::default();
        let req = TestRequest::get()
            .uri("/api/v1/namespaces")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous(), &registry).unwrap();

        assert_eq!(request.verb, "list");
        assert_eq!(request.resource, "namespaces");
        assert_eq!(request.resource_name, None);
        assert_eq!(request.namespace, None);
    }

    #[test]
    fn parses_cluster_scoped_namespace_read() {
        let registry = CrdRegistry::default();
        let req = TestRequest::get()
            .uri("/api/v1/namespaces/default")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous(), &registry).unwrap();

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
        let registry = CrdRegistry::default();
        for descriptor in resource_registry::all_resource_apis() {
            let collection_path = if descriptor.namespaced() {
                vec!["namespaces", "default", descriptor.plural]
            } else {
                vec![descriptor.plural]
            };
            let parsed = parse_resource_path(
                descriptor.group,
                descriptor.version,
                &collection_path,
                &registry,
            )
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
                let parsed = parse_resource_path(
                    descriptor.group,
                    descriptor.version,
                    &status_path,
                    &registry,
                )
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

    #[test]
    fn builds_authorization_request_for_namespaced_crd_resource() {
        let registry = registry_with_crd(CrdScope::Namespaced, true);
        let req = TestRequest::patch()
            .uri("/apis/example.com/v1/namespaces/default/widgets/demo/status")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous(), &registry).unwrap();

        assert_eq!(request.api_group, "example.com");
        assert_eq!(request.verb, "patch");
        assert_eq!(request.resource, "widgets/status");
        assert_eq!(request.resource_name.as_deref(), Some("demo"));
        assert_eq!(request.namespace.as_deref(), Some("default"));
    }

    #[test]
    fn builds_authorization_request_for_cluster_scoped_crd_resource() {
        let registry = registry_with_crd(CrdScope::Cluster, false);
        let req = TestRequest::get()
            .uri("/apis/example.com/v1/widgets/demo")
            .to_http_request();

        let request = build_authorization_request(&req, UserInfo::anonymous(), &registry).unwrap();

        assert_eq!(request.api_group, "example.com");
        assert_eq!(request.verb, "get");
        assert_eq!(request.resource, "widgets");
        assert_eq!(request.resource_name.as_deref(), Some("demo"));
        assert_eq!(request.namespace, None);
    }

    fn registry_with_crd(scope: CrdScope, status_subresource: bool) -> CrdRegistry {
        let registry = CrdRegistry::default();
        registry
            .upsert(CrdEntry {
                group: "example.com".to_string(),
                plural: "widgets".to_string(),
                singular: "widget".to_string(),
                kind: "Widget".to_string(),
                list_kind: "WidgetList".to_string(),
                scope,
                version: CrdVersionInfo {
                    name: "v1".to_string(),
                    served: true,
                    storage: true,
                    schema_json: None,
                    compiled_schema: None,
                    status_subresource,
                },
            })
            .expect("CRD registry update should succeed");
        registry
    }

    fn test_cert(subject_entries: &[(&str, &str)]) -> Vec<u8> {
        let mut name = DistinguishedName::new();
        for (key, value) in subject_entries {
            let kind = match *key {
                "CN" => DnType::CommonName,
                "O" => DnType::OrganizationName,
                other => panic!("unsupported subject field {other}"),
            };
            name.push(kind, *value);
        }
        let mut params = CertificateParams::default();
        params.distinguished_name = name;
        let key_pair = KeyPair::generate().expect("key pair should be generated");
        params
            .self_signed(&key_pair)
            .expect("certificate should be generated")
            .der()
            .to_vec()
    }

    const MULTIPLE_ORGANIZATIONS_CERT_DER_BASE64: &str = "\
        MIIBVDCB+gIJAMqcBymGI/jZMAoGCCqGSM49BAMCMDIxFDASBgNVBAMMC2NsaWVudC11c2Vy\
        MQwwCgYDVQQKDANvcHMxDDAKBgNVBAoMA3NyZTAeFw0yNjA5MDUwNjU1MzNaFw0yNjA5MDYw\
        NjU1MzNaMDIxFDASBgNVBAMMC2NsaWVudC11c2VyMQwwCgYDVQQKDANvcHMxDDAKBgNVBAoM\
        A3NyZTBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABOX0uBtIQbQzOCM3SptKAB+bKaCBP1J5\
        fTQ6o/tWwoSDif2aQzHY2kfKLZ9ry/wveHKmss3vFOjJKa5Y/TZWeAIwCgYIKoZIzj0EAwID\
        SQAwRgIhAO+raB9GJUimWbHgXwQDSCLck10AH654yYw2+I1ptOLOAiEAkeTisFEAV0ZcTpaT\
        LLfBrAaFMWLZRxK3DyL/igLysKM=";
}
