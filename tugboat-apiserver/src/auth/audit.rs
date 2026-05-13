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

use crate::auth::authorization::AuthorizationDecision;
use crate::auth::user_info::UserInfo;
use crate::config::{
    AUDIT_DEFAULT_MAX_AGE_DAYS, AUDIT_DEFAULT_MAX_SIZE_MB, AuditConfig, AuditLevel, AuditRule,
};
use crate::endpoints::resource_registry;
use actix_web::Error;
use actix_web::body::{BodySize, EitherBody, MessageBody};
use actix_web::dev::{Payload, Service, ServiceRequest, ServiceResponse, Transform, forward_ready};
use actix_web::http::Method;
use actix_web::web::Bytes;
use actix_web::{HttpMessage, HttpRequest};
use chrono::SecondsFormat;
use futures_util::StreamExt;
use serde::Serialize;
use std::collections::BTreeMap;
use std::future::{Future, Ready, ready};
use std::path::PathBuf;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::warn;
use uuid::Uuid;

/// Event payload that the audit middleware writes to the configured sink.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub(crate) struct AuditEvent {
    pub(crate) kind: &'static str,
    #[serde(rename = "apiVersion")]
    pub(crate) api_version: &'static str,
    pub(crate) level: AuditLevel,
    #[serde(rename = "auditID")]
    pub(crate) audit_id: String,
    pub(crate) stage: &'static str,
    #[serde(rename = "requestURI")]
    pub(crate) request_uri: String,
    pub(crate) verb: String,
    pub(crate) user: AuditUser,
    #[serde(rename = "sourceIPs", skip_serializing_if = "Vec::is_empty")]
    pub(crate) source_ips: Vec<String>,
    #[serde(rename = "userAgent", skip_serializing_if = "Option::is_none")]
    pub(crate) user_agent: Option<String>,
    #[serde(rename = "objectRef", skip_serializing_if = "Option::is_none")]
    pub(crate) object_ref: Option<ObjectRef>,
    #[serde(rename = "responseStatus")]
    pub(crate) response_status: ResponseStatus,
    #[serde(rename = "requestObject", skip_serializing_if = "Option::is_none")]
    pub(crate) request_object: Option<serde_json::Value>,
    #[serde(rename = "responseObject", skip_serializing_if = "Option::is_none")]
    pub(crate) response_object: Option<serde_json::Value>,
    #[serde(rename = "requestReceivedTimestamp")]
    pub(crate) request_received_timestamp: String,
    #[serde(rename = "stageTimestamp")]
    pub(crate) stage_timestamp: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) annotations: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub(crate) struct AuditUser {
    pub(crate) username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) uid: Option<String>,
    pub(crate) groups: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) extra: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct ObjectRef {
    pub(crate) resource: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(rename = "apiGroup")]
    pub(crate) api_group: String,
    #[serde(rename = "apiVersion")]
    pub(crate) api_version: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct ResponseStatus {
    pub(crate) code: u16,
}

/// Carries the level decided for a request to the response phase.
struct RequestContext {
    audit_id: String,
    level: AuditLevel,
    received_at: chrono::DateTime<chrono::Utc>,
    received_at_instant: Instant,
    request_uri: String,
    verb: String,
    object_ref: Option<ObjectRef>,
    user_agent: Option<String>,
    source_ips: Vec<String>,
}

/// Compiled policy used to pick a level for a request.
#[derive(Clone, Debug)]
pub(crate) struct AuditPolicy {
    rules: Vec<AuditRule>,
}

impl AuditPolicy {
    pub(crate) fn from_rules(rules: Vec<AuditRule>) -> Self {
        Self { rules }
    }

    /// Returns the first matching rule's level. With no rules, audit defaults to None.
    pub(crate) fn select_level(&self, request: &PolicyInput<'_>) -> AuditLevel {
        let level = self
            .rules
            .iter()
            .find(|rule| rule_matches(rule, request))
            .map(|rule| rule.level)
            .unwrap_or(AuditLevel::None);
        clamp_sensitive_level(request.resource, level)
    }
}

/// Hard fail-safe: Secret bodies must never reach the audit log even if an
/// operator's policy says `Request`/`RequestResponse`. Clamps the level to
/// `Metadata` (or `None`, whichever is lower) for the `secrets` resource and
/// any of its subresources.
fn clamp_sensitive_level(resource: &str, level: AuditLevel) -> AuditLevel {
    if strip_subresource(resource) != "secrets" {
        return level;
    }
    match level {
        AuditLevel::Request | AuditLevel::RequestResponse => AuditLevel::Metadata,
        other => other,
    }
}

/// Inputs the policy matcher inspects.
pub(crate) struct PolicyInput<'a> {
    pub(crate) verb: &'a str,
    pub(crate) user: &'a UserInfo,
    pub(crate) namespace: Option<&'a str>,
    pub(crate) api_group: &'a str,
    pub(crate) resource: &'a str,
    pub(crate) non_resource_url: Option<&'a str>,
}

fn rule_matches(rule: &AuditRule, request: &PolicyInput<'_>) -> bool {
    if !rule.verbs.is_empty() && !rule.verbs.iter().any(|v| v == request.verb) {
        return false;
    }
    if !rule.users.is_empty() && !rule.users.iter().any(|u| u == &request.user.username) {
        return false;
    }
    if !rule.user_groups.is_empty()
        && !rule
            .user_groups
            .iter()
            .any(|g| request.user.groups.iter().any(|ug| ug == g))
    {
        return false;
    }
    if !rule.namespaces.is_empty()
        && !rule
            .namespaces
            .iter()
            .any(|ns| Some(ns.as_str()) == request.namespace)
    {
        return false;
    }
    if !rule.non_resource_urls.is_empty() {
        let Some(url) = request.non_resource_url else {
            return false;
        };
        if !rule
            .non_resource_urls
            .iter()
            .any(|pattern| non_resource_url_matches(pattern, url))
        {
            return false;
        }
    }
    if !rule.resources.is_empty() {
        let request_resource = strip_subresource(request.resource);
        let matched = rule.resources.iter().any(|selector| {
            (selector.group.is_empty() || selector.group == request.api_group)
                && (selector.resources.is_empty()
                    || selector
                        .resources
                        .iter()
                        .any(|r| r == request.resource || r == request_resource || r == "*"))
        });
        if !matched {
            return false;
        }
    }
    true
}

fn strip_subresource(resource: &str) -> &str {
    resource
        .split_once('/')
        .map(|(base, _)| base)
        .unwrap_or(resource)
}

fn non_resource_url_matches(pattern: &str, url: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        url == prefix || url.starts_with(&format!("{prefix}/"))
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        url.starts_with(prefix)
    } else {
        pattern == url
    }
}

/// Handle used by middleware to send audit events to the writer task.
#[derive(Clone)]
pub(crate) struct AuditSink {
    sender: mpsc::Sender<AuditEvent>,
    dropped: Arc<AtomicU64>,
    max_request_body_bytes: usize,
    max_response_body_bytes: usize,
}

impl AuditSink {
    pub(crate) fn try_send(&self, event: AuditEvent) {
        if let Err(err) = self.sender.try_send(event) {
            match err {
                mpsc::error::TrySendError::Full(_) => {
                    let dropped = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                    if dropped.is_power_of_two() {
                        warn!(
                            target: "tugboat::audit",
                            dropped,
                            "audit channel is full; dropping events"
                        );
                    }
                }
                mpsc::error::TrySendError::Closed(_) => {
                    warn!(target: "tugboat::audit", "audit writer has stopped");
                }
            }
        }
    }

    pub(crate) fn max_request_body_bytes(&self) -> usize {
        self.max_request_body_bytes
    }

    pub(crate) fn max_response_body_bytes(&self) -> usize {
        self.max_response_body_bytes
    }
}

/// Starts the audit writer task. Returns a sink usable from middlewares.
pub(crate) fn start_audit_writer(config: &AuditConfig) -> Option<AuditSink> {
    if !config.enabled {
        return None;
    }
    warn_on_unsupported_audit_fields(config);
    let writer = match AuditWriter::open(config) {
        Ok(writer) => writer,
        Err(err) => {
            warn!(
                target: "tugboat::audit",
                error = %err,
                log_path = %config.log_path,
                "failed to open audit sink; audit logging will be disabled"
            );
            return None;
        }
    };
    let (sender, mut receiver) = mpsc::channel::<AuditEvent>(config.channel_capacity.max(1));
    let dropped = Arc::new(AtomicU64::new(0));

    tokio::spawn(async move {
        let mut writer = writer;
        while let Some(event) = receiver.recv().await {
            if let Err(err) = writer.write(&event) {
                warn!(
                    target: "tugboat::audit",
                    error = %err,
                    "failed to write audit event"
                );
            }
        }
    });

    Some(AuditSink {
        sender,
        dropped,
        max_request_body_bytes: config.max_request_body_bytes,
        max_response_body_bytes: config.max_response_body_bytes,
    })
}

/// `max_size_mb` and `max_age_days` are reserved for future use — the daily
/// rolling file appender does not honor them yet. Warn the operator when they
/// set non-default values so silent misconfiguration is caught at startup.
fn warn_on_unsupported_audit_fields(config: &AuditConfig) {
    if config.max_size_mb != AUDIT_DEFAULT_MAX_SIZE_MB {
        warn!(
            target: "tugboat::audit",
            max_size_mb = config.max_size_mb,
            "audit.max_size_mb is set but size-based rotation is not implemented yet; the value is ignored"
        );
    }
    if config.max_age_days != AUDIT_DEFAULT_MAX_AGE_DAYS {
        warn!(
            target: "tugboat::audit",
            max_age_days = config.max_age_days,
            "audit.max_age_days is set but age-based retention is not implemented yet; the value is ignored"
        );
    }
}

enum AuditWriter {
    Stdout,
    File(tracing_appender::rolling::RollingFileAppender),
}

impl AuditWriter {
    fn open(config: &AuditConfig) -> std::io::Result<Self> {
        if config.log_path == "-" {
            return Ok(Self::Stdout);
        }
        let path = PathBuf::from(&config.log_path);
        let directory = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let file_name = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "audit.log".to_string());
        std::fs::create_dir_all(&directory)?;
        let appender = tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix(file_name)
            .max_log_files(config.max_backups.max(1))
            .build(&directory)
            .map_err(std::io::Error::other)?;
        Ok(Self::File(appender))
    }

    fn write(&mut self, event: &AuditEvent) -> std::io::Result<()> {
        use std::io::Write;
        let line = serde_json::to_string(event)
            .map_err(|e| std::io::Error::other(format!("serialize audit event: {e}")))?;
        match self {
            Self::Stdout => {
                let mut out = std::io::stdout().lock();
                out.write_all(line.as_bytes())?;
                out.write_all(b"\n")?;
            }
            Self::File(appender) => {
                appender.write_all(line.as_bytes())?;
                appender.write_all(b"\n")?;
                appender.flush()?;
            }
        }
        Ok(())
    }
}

/// Strips the port from a `realip_remote_addr()` value while preserving IPv6
/// addresses. Handles `10.0.0.1:43210`, `[::1]:43210`, and bare addresses
/// without a port (`10.0.0.1`, `::1`).
pub(crate) fn parse_remote_ip(addr: &str) -> String {
    if let Ok(socket) = addr.parse::<std::net::SocketAddr>() {
        return socket.ip().to_string();
    }
    if let Ok(ip) = addr.parse::<std::net::IpAddr>() {
        return ip.to_string();
    }
    // Bracketed IPv6 with non-numeric trailer or other unusual shape.
    if let Some(rest) = addr.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        return rest[..end].to_string();
    }
    // Last-colon split is only safe when the left side has no colons
    // (i.e. cannot be an IPv6 address).
    match addr.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') && port.chars().all(|c| c.is_ascii_digit()) => {
            host.to_string()
        }
        _ => addr.to_string(),
    }
}

/// HTTP verb mapping consistent with Kubernetes audit semantics.
pub(crate) fn audit_verb(method: &Method, query: &str, resource_name_present: bool) -> String {
    match *method {
        Method::GET if query_requests_watch(query) => "watch".to_string(),
        Method::GET if resource_name_present => "get".to_string(),
        Method::GET => "list".to_string(),
        Method::POST => "create".to_string(),
        Method::PUT => "update".to_string(),
        Method::PATCH => "patch".to_string(),
        Method::DELETE if resource_name_present => "delete".to_string(),
        Method::DELETE => "deletecollection".to_string(),
        Method::HEAD => "head".to_string(),
        Method::OPTIONS => "options".to_string(),
        _ => method.as_str().to_ascii_lowercase(),
    }
}

fn query_requests_watch(query: &str) -> bool {
    query.split('&').any(|pair| {
        let Some((key, value)) = pair.split_once('=') else {
            return false;
        };
        key == "watch" && matches!(value, "true" | "True" | "ndJson" | "NdJson")
    })
}

/// Parses a resource path the same way the authorization middleware does and
/// returns the audit-shaped (api_group, version, ObjectRef) triple.
pub(crate) fn object_ref_for_request(req: &HttpRequest) -> ParsedRequest {
    let path = req.path();
    let mut segments = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty());

    let (api_group, version) = match segments.next() {
        Some("api") => match segments.next() {
            Some(v) => ("".to_string(), v.to_string()),
            None => return ParsedRequest::non_resource(path),
        },
        Some("apis") => match (segments.next(), segments.next()) {
            (Some(g), Some(v)) => (g.to_string(), v.to_string()),
            _ => return ParsedRequest::non_resource(path),
        },
        _ => return ParsedRequest::non_resource(path),
    };

    let remaining = segments.collect::<Vec<_>>();
    let parsed = match parse_resource_path(api_group.as_str(), version.as_str(), &remaining) {
        Some(parsed) => parsed,
        None => return ParsedRequest::non_resource(path),
    };

    ParsedRequest {
        api_group: api_group.clone(),
        object_ref: Some(ObjectRef {
            resource: parsed.resource,
            namespace: parsed.namespace,
            name: parsed.resource_name,
            api_group,
            api_version: version,
        }),
        non_resource_url: None,
        resource_name_present: parsed.name_present,
    }
}

pub(crate) struct ParsedRequest {
    pub(crate) api_group: String,
    pub(crate) object_ref: Option<ObjectRef>,
    pub(crate) non_resource_url: Option<String>,
    pub(crate) resource_name_present: bool,
}

impl ParsedRequest {
    fn non_resource(path: &str) -> Self {
        Self {
            api_group: String::new(),
            object_ref: None,
            non_resource_url: Some(path.to_string()),
            resource_name_present: false,
        }
    }
}

struct ParsedResourcePath {
    resource: String,
    resource_name: Option<String>,
    namespace: Option<String>,
    name_present: bool,
}

fn parse_resource_path(
    api_group: &str,
    version: &str,
    path: &[&str],
) -> Option<ParsedResourcePath> {
    // Convert legacy ""/v1 (used in audit) into the registry's "core"/v1 group key.
    let registry_group = if api_group.is_empty() {
        "core"
    } else {
        api_group
    };
    let descriptors = resource_registry::resources_for(registry_group, version);
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
    let resource_name = after_resource.first().map(|s| (*s).to_string());
    let name_present = resource_name.is_some();
    let resource = if after_resource.len() > 1 {
        format!("{resource}/{}", after_resource[1..].join("/"))
    } else {
        resource.to_string()
    };

    Some(ParsedResourcePath {
        resource,
        resource_name,
        namespace: path_namespace,
        name_present,
    })
}

/// Decision recorded by the authorization middleware so the audit middleware can
/// annotate the event with the allow/deny reason.
#[derive(Clone, Debug)]
pub(crate) struct RecordedAuthorizationDecision(pub(crate) AuthorizationDecision);

#[derive(Clone)]
pub(crate) struct AuditMiddleware {
    policy: Arc<AuditPolicy>,
    sink: Option<AuditSink>,
}

impl AuditMiddleware {
    pub(crate) fn new(policy: Arc<AuditPolicy>, sink: Option<AuditSink>) -> Self {
        Self { policy, sink }
    }
}

impl<S, B> Transform<S, ServiceRequest> for AuditMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<CapturedBody<B>, B>>;
    type Error = Error;
    type InitError = ();
    type Transform = AuditMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuditMiddlewareService {
            service: Rc::new(service),
            policy: self.policy.clone(),
            sink: self.sink.clone(),
        }))
    }
}

pub(crate) struct AuditMiddlewareService<S> {
    service: Rc<S>,
    policy: Arc<AuditPolicy>,
    sink: Option<AuditSink>,
}

impl<S, B> Service<ServiceRequest> for AuditMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<CapturedBody<B>, B>>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    forward_ready!(service);

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let service = Rc::clone(&self.service);
        let policy = self.policy.clone();
        let sink = self.sink.clone();

        Box::pin(async move {
            let Some(sink) = sink else {
                let res = service.call(req).await?;
                return Ok(res.map_into_right_body());
            };

            let user = req
                .extensions()
                .get::<UserInfo>()
                .cloned()
                .unwrap_or_else(UserInfo::anonymous);
            let parsed = object_ref_for_request(req.request());
            let verb = audit_verb(
                req.method(),
                req.query_string(),
                parsed.resource_name_present,
            );

            let policy_input = PolicyInput {
                verb: verb.as_str(),
                user: &user,
                namespace: parsed
                    .object_ref
                    .as_ref()
                    .and_then(|r| r.namespace.as_deref()),
                api_group: parsed.api_group.as_str(),
                resource: parsed
                    .object_ref
                    .as_ref()
                    .map(|r| r.resource.as_str())
                    .unwrap_or(""),
                non_resource_url: parsed.non_resource_url.as_deref(),
            };
            let level = policy.select_level(&policy_input);

            if matches!(level, AuditLevel::None) {
                let res = service.call(req).await?;
                return Ok(res.map_into_right_body());
            }

            let audit_id = Uuid::new_v4().to_string();
            let received_at = chrono::Utc::now();
            let received_at_instant = Instant::now();
            let user_agent = req
                .headers()
                .get(actix_web::http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string());
            let source_ips = req
                .connection_info()
                .realip_remote_addr()
                .map(parse_remote_ip)
                .map(|ip| vec![ip])
                .unwrap_or_default();
            let request_uri = match req.uri().query() {
                Some(q) if !q.is_empty() => format!("{}?{}", req.path(), q),
                _ => req.path().to_string(),
            };

            let mut annotations = BTreeMap::new();
            let mut request_object: Option<serde_json::Value> = None;

            // Capture request body if needed.
            if matches!(level, AuditLevel::Request | AuditLevel::RequestResponse)
                && sink.max_request_body_bytes() > 0
            {
                let (body, truncated) =
                    drain_payload(&mut req, sink.max_request_body_bytes()).await?;
                if truncated {
                    annotations.insert(
                        "audit.tugboat.cloud/request-body-truncated".to_string(),
                        "true".to_string(),
                    );
                }
                request_object = Some(body_to_value(body));
            }

            let context = RequestContext {
                audit_id,
                level,
                received_at,
                received_at_instant,
                request_uri,
                verb,
                object_ref: parsed.object_ref,
                user_agent,
                source_ips,
            };

            let user_clone = user.clone();
            let res = service.call(req).await?;

            // Pull authorization decision (if recorded) for the annotations.
            if let Some(decision) = res
                .request()
                .extensions()
                .get::<RecordedAuthorizationDecision>()
                .cloned()
            {
                match decision.0 {
                    AuthorizationDecision::Allowed => {
                        annotations.insert(
                            "authorization.tugboat.cloud/decision".to_string(),
                            "allow".to_string(),
                        );
                    }
                    AuthorizationDecision::Denied { reason } => {
                        annotations.insert(
                            "authorization.tugboat.cloud/decision".to_string(),
                            "deny".to_string(),
                        );
                        annotations
                            .insert("authorization.tugboat.cloud/reason".to_string(), reason);
                    }
                }
            }

            let status_code = res.status().as_u16();
            let capture_response =
                matches!(level, AuditLevel::RequestResponse) && sink.max_response_body_bytes() > 0;

            let context = Arc::new(context);
            let user_audit = audit_user(&user_clone);
            let sink_for_body = sink.clone();
            let annotations_for_body = annotations.clone();
            let request_object_for_body = request_object.clone();

            let res = res.map_body(|_head, body| {
                if !capture_response {
                    // Even when we don't capture the body, we still need to emit
                    // the audit event after the response finishes streaming.
                    return EitherBody::left(CapturedBody {
                        inner: body,
                        captured: Vec::new(),
                        limit: 0,
                        truncated: false,
                        completed: false,
                        on_complete: Some(BodyCompletion {
                            context: context.clone(),
                            sink: sink_for_body.clone(),
                            user: user_audit.clone(),
                            status_code,
                            annotations: annotations_for_body.clone(),
                            request_object: request_object_for_body.clone(),
                            capture_response: false,
                        }),
                    });
                }
                EitherBody::left(CapturedBody {
                    inner: body,
                    captured: Vec::new(),
                    limit: sink_for_body.max_response_body_bytes(),
                    truncated: false,
                    completed: false,
                    on_complete: Some(BodyCompletion {
                        context: context.clone(),
                        sink: sink_for_body.clone(),
                        user: user_audit.clone(),
                        status_code,
                        annotations: annotations_for_body.clone(),
                        request_object: request_object_for_body.clone(),
                        capture_response: true,
                    }),
                })
            });

            Ok(res)
        })
    }
}

fn audit_user(user: &UserInfo) -> AuditUser {
    let mut extra = BTreeMap::new();
    for (k, v) in &user.extra {
        extra.insert(k.clone(), v.clone());
    }
    AuditUser {
        username: user.username.clone(),
        uid: user.uid.clone(),
        groups: user.groups.clone(),
        extra,
    }
}

async fn drain_payload(req: &mut ServiceRequest, limit: usize) -> Result<(Vec<u8>, bool), Error> {
    use actix_web::error::PayloadError;
    use futures_util::Stream;

    let mut payload = req.take_payload();
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(actix_web::error::ErrorInternalServerError)?;
        if buf.len() >= limit {
            truncated = true;
            continue;
        }
        let take = limit.saturating_sub(buf.len()).min(chunk.len());
        buf.extend_from_slice(&chunk[..take]);
        if take < chunk.len() {
            truncated = true;
        }
    }
    let stored = Bytes::copy_from_slice(&buf);
    let stream = futures_util::stream::once(async move { Ok::<Bytes, PayloadError>(stored) });
    let boxed: Pin<Box<dyn Stream<Item = Result<Bytes, PayloadError>>>> = Box::pin(stream);
    req.set_payload(Payload::from(boxed));
    Ok((buf, truncated))
}

fn body_to_value(bytes: Vec<u8>) -> serde_json::Value {
    if bytes.is_empty() {
        return serde_json::Value::Null;
    }
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => value,
        Err(_) => match std::str::from_utf8(&bytes) {
            Ok(text) => serde_json::Value::String(text.to_string()),
            Err(_) => serde_json::Value::String(format!("<binary {} bytes>", bytes.len())),
        },
    }
}

#[derive(Clone)]
struct BodyCompletion {
    context: Arc<RequestContext>,
    sink: AuditSink,
    user: AuditUser,
    status_code: u16,
    annotations: BTreeMap<String, String>,
    request_object: Option<serde_json::Value>,
    capture_response: bool,
}

pub(crate) struct CapturedBody<B> {
    inner: B,
    captured: Vec<u8>,
    limit: usize,
    truncated: bool,
    completed: bool,
    on_complete: Option<BodyCompletion>,
}

impl<B: MessageBody> MessageBody for CapturedBody<B> {
    type Error = B::Error;

    fn size(&self) -> BodySize {
        self.inner.size()
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        // SAFETY: structural projection of fields we own; inner is the only pinned field.
        let this = unsafe { self.get_unchecked_mut() };
        let inner = unsafe { Pin::new_unchecked(&mut this.inner) };
        match inner.poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                if this.limit > 0 && this.captured.len() < this.limit {
                    let take = this
                        .limit
                        .saturating_sub(this.captured.len())
                        .min(chunk.len());
                    this.captured.extend_from_slice(&chunk[..take]);
                    if take < chunk.len() {
                        this.truncated = true;
                    }
                } else if this.limit > 0 {
                    this.truncated = true;
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(None) => {
                if !this.completed {
                    this.completed = true;
                    emit_completion(this);
                }
                Poll::Ready(None)
            }
            other => other,
        }
    }
}

impl<B> Drop for CapturedBody<B> {
    fn drop(&mut self) {
        if !self.completed {
            self.completed = true;
            emit_completion(self);
        }
    }
}

fn emit_completion<B>(body: &mut CapturedBody<B>) {
    let Some(completion) = body.on_complete.take() else {
        return;
    };

    let mut annotations = completion.annotations;
    let response_object = if completion.capture_response {
        if body.truncated {
            annotations.insert(
                "audit.tugboat.cloud/response-body-truncated".to_string(),
                "true".to_string(),
            );
        }
        Some(body_to_value(std::mem::take(&mut body.captured)))
    } else {
        None
    };

    let stage_timestamp = chrono::Utc::now();
    let elapsed_us = completion.context.received_at_instant.elapsed().as_micros();
    annotations.insert(
        "audit.tugboat.cloud/latency-microseconds".to_string(),
        elapsed_us.to_string(),
    );

    let event = AuditEvent {
        kind: "Event",
        api_version: "audit.tugboat.cloud/v1",
        level: completion.context.level,
        audit_id: completion.context.audit_id.clone(),
        stage: "ResponseComplete",
        request_uri: completion.context.request_uri.clone(),
        verb: completion.context.verb.clone(),
        user: completion.user,
        source_ips: completion.context.source_ips.clone(),
        user_agent: completion.context.user_agent.clone(),
        object_ref: completion.context.object_ref.clone(),
        response_status: ResponseStatus {
            code: completion.status_code,
        },
        request_object: completion.request_object,
        response_object,
        request_received_timestamp: completion
            .context
            .received_at
            .to_rfc3339_opts(SecondsFormat::Micros, true),
        stage_timestamp: stage_timestamp.to_rfc3339_opts(SecondsFormat::Micros, true),
        annotations,
    };
    completion.sink.try_send(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuditResourceSelector, AuditRule};
    use std::collections::HashMap;

    fn user() -> UserInfo {
        UserInfo::x509(
            "alice".to_string(),
            vec!["developers".to_string()],
            HashMap::new(),
        )
    }

    fn input<'a>(
        verb: &'a str,
        user: &'a UserInfo,
        api_group: &'a str,
        resource: &'a str,
        namespace: Option<&'a str>,
    ) -> PolicyInput<'a> {
        PolicyInput {
            verb,
            user,
            namespace,
            api_group,
            resource,
            non_resource_url: None,
        }
    }

    #[test]
    fn empty_policy_returns_none() {
        let policy = AuditPolicy::from_rules(vec![]);
        let u = user();
        let level = policy.select_level(&input("get", &u, "core", "ships", None));
        assert_eq!(level, AuditLevel::None);
    }

    #[test]
    fn first_matching_rule_wins() {
        let policy = AuditPolicy::from_rules(vec![
            AuditRule {
                level: AuditLevel::None,
                resources: vec![AuditResourceSelector {
                    group: "coordination".to_string(),
                    resources: vec!["leases".to_string()],
                }],
                ..Default::default()
            },
            AuditRule {
                level: AuditLevel::RequestResponse,
                verbs: vec!["create".to_string()],
                ..Default::default()
            },
            AuditRule {
                level: AuditLevel::Metadata,
                ..Default::default()
            },
        ]);
        let u = user();
        assert_eq!(
            policy.select_level(&input("update", &u, "coordination", "leases", None)),
            AuditLevel::None
        );
        assert_eq!(
            policy.select_level(&input("create", &u, "core", "ships", Some("default"))),
            AuditLevel::RequestResponse
        );
        assert_eq!(
            policy.select_level(&input("get", &u, "core", "ships", Some("default"))),
            AuditLevel::Metadata
        );
    }

    #[test]
    fn user_filter_requires_username_match() {
        let policy = AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::None,
            users: vec!["system:serviceaccount:kube-system:scheduler".to_string()],
            ..Default::default()
        }]);
        let scheduler = UserInfo::service_account("kube-system", "scheduler", None, HashMap::new());
        let alice = user();
        assert_eq!(
            policy.select_level(&input("update", &scheduler, "coordination", "leases", None)),
            AuditLevel::None
        );
        assert_eq!(
            policy.select_level(&input("update", &alice, "coordination", "leases", None)),
            AuditLevel::None
        ); // Default fallthrough is also None when no rule matches.
    }

    #[test]
    fn group_filter_matches_membership() {
        let policy = AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::RequestResponse,
            user_groups: vec!["developers".to_string()],
            ..Default::default()
        }]);
        let u = user();
        assert_eq!(
            policy.select_level(&input("get", &u, "core", "ships", None)),
            AuditLevel::RequestResponse
        );
    }

    #[test]
    fn namespace_filter_matches_target_namespace() {
        let policy = AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::RequestResponse,
            namespaces: vec!["sensitive".to_string()],
            ..Default::default()
        }]);
        let u = user();
        assert_eq!(
            policy.select_level(&input("get", &u, "core", "ships", Some("sensitive"))),
            AuditLevel::RequestResponse
        );
        assert_eq!(
            policy.select_level(&input("get", &u, "core", "ships", Some("default"))),
            AuditLevel::None
        );
    }

    #[test]
    fn secret_bodies_are_clamped_to_metadata_regardless_of_policy() {
        let policy = AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::RequestResponse,
            ..Default::default()
        }]);
        let u = user();
        // Plain secrets resource is clamped down from RequestResponse.
        assert_eq!(
            policy.select_level(&input("create", &u, "core", "secrets", Some("default"))),
            AuditLevel::Metadata
        );
        // Subresources of secrets (none in tugboat today, but defensive) are
        // also clamped via the base-resource check.
        assert_eq!(
            policy.select_level(&input("get", &u, "core", "secrets/data", Some("default"))),
            AuditLevel::Metadata
        );
        // Non-secret resources are untouched.
        assert_eq!(
            policy.select_level(&input("create", &u, "core", "ships", Some("default"))),
            AuditLevel::RequestResponse
        );
    }

    #[test]
    fn secret_none_level_stays_none_when_clamped() {
        let policy = AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::None,
            ..Default::default()
        }]);
        let u = user();
        assert_eq!(
            policy.select_level(&input("create", &u, "core", "secrets", Some("default"))),
            AuditLevel::None
        );
    }

    #[test]
    fn subresource_resource_matches_base() {
        let policy = AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::Metadata,
            resources: vec![AuditResourceSelector {
                group: "core".to_string(),
                resources: vec!["serviceaccounts".to_string()],
            }],
            ..Default::default()
        }]);
        let u = user();
        assert_eq!(
            policy.select_level(&input(
                "create",
                &u,
                "core",
                "serviceaccounts/token",
                Some("default"),
            )),
            AuditLevel::Metadata
        );
    }

    #[test]
    fn non_resource_url_matches_prefix() {
        let policy = AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::Metadata,
            non_resource_urls: vec!["/healthz".to_string(), "/openapi/*".to_string()],
            ..Default::default()
        }]);
        let u = user();
        let select = |url: &str| {
            policy.select_level(&PolicyInput {
                verb: "get",
                user: &u,
                namespace: None,
                api_group: "",
                resource: "",
                non_resource_url: Some(url),
            })
        };
        assert_eq!(select("/healthz"), AuditLevel::Metadata);
        assert_eq!(select("/openapi/v3"), AuditLevel::Metadata);
        assert_eq!(select("/openapi/v3/api"), AuditLevel::Metadata);
        assert_eq!(select("/other"), AuditLevel::None);
    }

    #[test]
    fn parse_remote_ip_handles_ipv4_ipv6_and_bare_addresses() {
        assert_eq!(parse_remote_ip("10.0.0.5:43210"), "10.0.0.5");
        assert_eq!(parse_remote_ip("[::1]:43210"), "::1");
        assert_eq!(parse_remote_ip("[2001:db8::1]:443"), "2001:db8::1");
        // Bare IPs (no port) should be returned untouched, not truncated.
        assert_eq!(parse_remote_ip("10.0.0.5"), "10.0.0.5");
        assert_eq!(parse_remote_ip("::1"), "::1");
        assert_eq!(parse_remote_ip("2001:db8::1"), "2001:db8::1");
    }

    #[test]
    fn verb_mapping_follows_kubernetes_semantics() {
        assert_eq!(audit_verb(&Method::GET, "", false), "list");
        assert_eq!(audit_verb(&Method::GET, "", true), "get");
        assert_eq!(audit_verb(&Method::GET, "watch=true", false), "watch");
        assert_eq!(audit_verb(&Method::POST, "", false), "create");
        assert_eq!(audit_verb(&Method::PUT, "", true), "update");
        assert_eq!(audit_verb(&Method::PATCH, "", true), "patch");
        assert_eq!(audit_verb(&Method::DELETE, "", true), "delete");
        assert_eq!(audit_verb(&Method::DELETE, "", false), "deletecollection");
    }

    #[test]
    fn object_ref_extracts_namespace_and_name() {
        let req = actix_web::test::TestRequest::default()
            .uri("/api/v1/namespaces/default/configmaps/example")
            .to_http_request();
        let parsed = object_ref_for_request(&req);
        let object_ref = parsed.object_ref.expect("object ref should be extracted");
        assert_eq!(object_ref.resource, "configmaps");
        assert_eq!(object_ref.namespace.as_deref(), Some("default"));
        assert_eq!(object_ref.name.as_deref(), Some("example"));
        assert_eq!(object_ref.api_group, "");
        assert_eq!(object_ref.api_version, "v1");
        assert!(parsed.non_resource_url.is_none());
    }

    #[test]
    fn object_ref_handles_subresource() {
        let req = actix_web::test::TestRequest::default()
            .uri("/apis/apps/v1/namespaces/default/deployments/example/scale")
            .to_http_request();
        let parsed = object_ref_for_request(&req);
        let object_ref = parsed.object_ref.expect("object ref should be extracted");
        assert_eq!(object_ref.resource, "deployments/scale");
        assert_eq!(object_ref.api_group, "apps");
    }

    #[test]
    fn object_ref_falls_back_to_non_resource_url() {
        let req = actix_web::test::TestRequest::default()
            .uri("/healthz")
            .to_http_request();
        let parsed = object_ref_for_request(&req);
        assert!(parsed.object_ref.is_none());
        assert_eq!(parsed.non_resource_url.as_deref(), Some("/healthz"));
    }

    #[test]
    fn audit_event_serializes_kubernetes_shape() {
        let event = AuditEvent {
            kind: "Event",
            api_version: "audit.tugboat.cloud/v1",
            level: AuditLevel::Metadata,
            audit_id: "f7c4".to_string(),
            stage: "ResponseComplete",
            request_uri: "/api/v1/namespaces/default/ships".to_string(),
            verb: "list".to_string(),
            user: AuditUser {
                username: "alice".to_string(),
                uid: None,
                groups: vec!["system:authenticated".to_string()],
                extra: BTreeMap::new(),
            },
            source_ips: vec!["10.0.0.5".to_string()],
            user_agent: Some("tugboat-client/0.1.0".to_string()),
            object_ref: Some(ObjectRef {
                resource: "ships".to_string(),
                namespace: Some("default".to_string()),
                name: None,
                api_group: "".to_string(),
                api_version: "v1".to_string(),
            }),
            response_status: ResponseStatus { code: 200 },
            request_object: None,
            response_object: None,
            request_received_timestamp: "2026-05-11T10:00:00.000000Z".to_string(),
            stage_timestamp: "2026-05-11T10:00:00.012000Z".to_string(),
            annotations: BTreeMap::from([(
                "authorization.tugboat.cloud/decision".to_string(),
                "allow".to_string(),
            )]),
        };

        let value = serde_json::to_value(&event).expect("event should serialize");
        assert_eq!(value["kind"], "Event");
        assert_eq!(value["apiVersion"], "audit.tugboat.cloud/v1");
        assert_eq!(value["auditID"], "f7c4");
        assert_eq!(value["stage"], "ResponseComplete");
        assert_eq!(value["requestURI"], "/api/v1/namespaces/default/ships");
        assert_eq!(value["verb"], "list");
        assert_eq!(value["user"]["username"], "alice");
        assert_eq!(value["sourceIPs"][0], "10.0.0.5");
        assert_eq!(value["objectRef"]["resource"], "ships");
        assert_eq!(value["objectRef"]["namespace"], "default");
        assert!(value["objectRef"].get("name").is_none());
        assert_eq!(value["responseStatus"]["code"], 200);
        assert_eq!(
            value["annotations"]["authorization.tugboat.cloud/decision"],
            "allow"
        );
        // Optional fields with None must be omitted entirely.
        assert!(value.get("requestObject").is_none());
        assert!(value.get("responseObject").is_none());
    }

    #[actix_web::test]
    async fn middleware_emits_event_on_response_complete() {
        use actix_web::{App, HttpResponse, test, web};

        let policy = Arc::new(AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::Metadata,
            ..Default::default()
        }]));
        let (sender, mut receiver) = mpsc::channel::<AuditEvent>(8);
        let sink = AuditSink {
            sender,
            dropped: Arc::new(AtomicU64::new(0)),
            max_request_body_bytes: 0,
            max_response_body_bytes: 0,
        };

        let app = test::init_service(
            App::new()
                .wrap(AuditMiddleware::new(policy.clone(), Some(sink.clone())))
                .route(
                    "/api/v1/namespaces/default/ships",
                    web::get().to(|| async { HttpResponse::Ok().body("ok") }),
                ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/api/v1/namespaces/default/ships")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        // Drop the response body so CapturedBody's Drop emits the event.
        drop(resp);

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("audit event should be produced")
            .expect("channel must yield an event");
        assert_eq!(event.verb, "list");
        assert_eq!(event.response_status.code, 200);
        assert_eq!(event.level, AuditLevel::Metadata);
        let object_ref = event.object_ref.expect("object ref present");
        assert_eq!(object_ref.resource, "ships");
        assert_eq!(object_ref.namespace.as_deref(), Some("default"));
    }

    #[actix_web::test]
    async fn middleware_skips_when_sink_is_disabled() {
        use actix_web::{App, HttpResponse, test, web};

        let policy = Arc::new(AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::Metadata,
            ..Default::default()
        }]));
        let app = test::init_service(
            App::new()
                .wrap(AuditMiddleware::new(policy.clone(), None))
                .route(
                    "/api/v1/namespaces/default/ships",
                    web::get().to(|| async { HttpResponse::Ok().body("ok") }),
                ),
        )
        .await;
        let req = test::TestRequest::get()
            .uri("/api/v1/namespaces/default/ships")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }

    #[actix_web::test]
    async fn middleware_records_request_and_response_bodies() {
        use actix_web::{App, HttpRequest, HttpResponse, test, web};
        use bytes::Bytes;

        let policy = Arc::new(AuditPolicy::from_rules(vec![AuditRule {
            level: AuditLevel::RequestResponse,
            ..Default::default()
        }]));
        let (sender, mut receiver) = mpsc::channel::<AuditEvent>(8);
        let sink = AuditSink {
            sender,
            dropped: Arc::new(AtomicU64::new(0)),
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
        };

        async fn echo(_req: HttpRequest, body: Bytes) -> HttpResponse {
            HttpResponse::Ok()
                .content_type("application/json")
                .body(body)
        }

        let app = test::init_service(
            App::new()
                .wrap(AuditMiddleware::new(policy.clone(), Some(sink.clone())))
                .route(
                    "/api/v1/namespaces/default/configmaps",
                    web::post().to(echo),
                ),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/api/v1/namespaces/default/configmaps")
            .set_payload(r#"{"name":"alpha"}"#)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let _ = test::read_body(resp).await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("audit event should be produced")
            .expect("channel must yield an event");
        assert_eq!(event.verb, "create");
        assert_eq!(event.level, AuditLevel::RequestResponse);
        let request_object = event.request_object.expect("request body captured");
        assert_eq!(request_object["name"], "alpha");
        let response_object = event.response_object.expect("response body captured");
        assert_eq!(response_object["name"], "alpha");
    }
}
