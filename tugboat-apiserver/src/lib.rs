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

use crate::auth::audit::{AuditMiddleware, AuditPolicy, start_audit_writer};
use crate::auth::middleware::{
    AuthenticationMiddleware, AuthorizationMiddleware, ClientCertificateInfo,
};
use crate::config::{AuditConfig, AuthenticationConfig, AuthorizationConfig, TlsConfig};
use crate::data::StatusResponse;
use crate::operator::ApiOperator;
use actix_web::error::InternalError;
use actix_web::middleware::Logger;
use actix_web::web::{Data, JsonConfig};
use actix_web::{App, HttpResponse, HttpServer, dev::Extensions, get};
use rustls::RootCertStore;
use rustls::server::WebPkiClientVerifier;
use rustls::{ServerConfig, crypto};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::any::Any;
use std::net::TcpListener;
use std::sync::Arc;
use tracing::warn;
use utoipa_actix_web::AppExt;

pub mod auth;
pub mod config;
pub mod crd_registry;
pub(crate) mod crd_schema;
mod data;
mod endpoints;
mod name_generator;
mod operator;

pub struct ApiServer {
    listen: String,
    operator: ApiOperator,
    tls: Option<TlsConfig>,
    authentication: AuthenticationConfig,
    authorization: AuthorizationConfig,
    audit: AuditConfig,
    allow_insecure_http: bool,
}

impl ApiServer {
    fn new(
        listen: String,
        operator: ApiOperator,
        tls: Option<TlsConfig>,
        authentication: AuthenticationConfig,
        authorization: AuthorizationConfig,
        audit: AuditConfig,
        allow_insecure_http: bool,
    ) -> Self {
        Self {
            listen,
            operator,
            tls,
            authentication,
            authorization,
            audit,
            allow_insecure_http,
        }
    }

    pub async fn run(self) -> std::io::Result<()> {
        crate::auth::bootstrap::bootstrap_default_rbac(&self.operator.store)
            .await
            .map_err(|e| {
                std::io::Error::other(format!("Failed to bootstrap default RBAC resources: {e}"))
            })?;
        crd_registry::load_crds_into_registry(&self.operator.store, &self.operator.crd_registry)
            .await
            .map(|_| ())
            .map_err(|e| {
                std::io::Error::other(format!("Failed to load CRDs into registry: {e}"))
            })?;
        let crd_registry = self.operator.crd_registry.clone();
        let crd_registry_data = Data::from(crd_registry.clone());
        tokio::spawn(crd_registry::run_crd_watcher(
            self.operator.store.clone(),
            crd_registry,
        ));
        let data = Data::new(self.operator);
        let authentication = self.authentication.clone();
        let authorization = self.authorization.clone();
        let audit_policy = Arc::new(AuditPolicy::from_rules(self.audit.rules.clone()));
        let audit_sink = start_audit_writer(&self.audit);
        let server = HttpServer::new(move || {
            App::new()
                .wrap(AuthorizationMiddleware::new(
                    data.clone(),
                    authorization.clone(),
                ))
                .wrap(AuditMiddleware::new(
                    audit_policy.clone(),
                    audit_sink.clone(),
                ))
                .wrap(AuthenticationMiddleware::new(
                    data.clone(),
                    authentication.clone(),
                ))
                .wrap(Logger::default().exclude("/healthz"))
                .app_data(data.clone())
                .app_data(crd_registry_data.clone())
                .app_data(json_config())
                .service(health_check)
                .configure(endpoints::register_openapi_endpoints)
                .into_utoipa_app()
                .configure(endpoints::register_endpoints)
                .into_app()
        })
        .on_connect(store_client_certificate_info);
        if let Some(tls) = self.tls {
            let config = build_tls_server_config(tls).map_err(std::io::Error::other)?;
            server.bind_rustls_0_23(self.listen, config)?.run().await
        } else if self.allow_insecure_http {
            warn!(
                "API server is running without TLS. This is insecure and not recommended for production use."
            );
            server.bind(self.listen)?.run().await
        } else {
            Err(std::io::Error::other(
                "TLS configuration is missing and allow_insecure_http is false. Refusing to start in insecure mode.",
            ))
        }
    }

    pub async fn run_with_listener(self, listener: TcpListener) {
        let Self {
            operator,
            tls,
            authentication,
            authorization,
            audit,
            allow_insecure_http,
            ..
        } = self;
        run_with_bound_listener(
            operator,
            authentication,
            authorization,
            audit,
            listener,
            tls,
            allow_insecure_http,
        )
        .await;
    }

    pub async fn run_with_tls_listener(self, listener: TcpListener, tls: TlsConfig) {
        let Self {
            operator,
            authentication,
            authorization,
            audit,
            ..
        } = self;
        run_with_bound_listener(
            operator,
            authentication,
            authorization,
            audit,
            listener,
            Some(tls),
            false,
        )
        .await;
    }
}

async fn run_with_bound_listener(
    operator: ApiOperator,
    authentication: AuthenticationConfig,
    authorization: AuthorizationConfig,
    audit: AuditConfig,
    listener: TcpListener,
    tls: Option<TlsConfig>,
    allow_insecure_http: bool,
) {
    if let Err(err) = crate::auth::bootstrap::bootstrap_default_rbac(&operator.store).await {
        tracing::error!("Failed to bootstrap default RBAC resources: {err}");
        return;
    }
    if let Err(err) =
        crd_registry::load_crds_into_registry(&operator.store, &operator.crd_registry).await
    {
        tracing::error!("Failed to load CRDs into registry: {err}");
        return;
    }
    let crd_registry = operator.crd_registry.clone();
    let crd_registry_data = Data::from(crd_registry.clone());
    tokio::spawn(crd_registry::run_crd_watcher(
        operator.store.clone(),
        crd_registry,
    ));
    let data = Data::new(operator);
    let audit_policy = Arc::new(AuditPolicy::from_rules(audit.rules.clone()));
    let audit_sink = start_audit_writer(&audit);
    let server = HttpServer::new(move || {
        App::new()
            .wrap(AuthorizationMiddleware::new(
                data.clone(),
                authorization.clone(),
            ))
            .wrap(AuditMiddleware::new(
                audit_policy.clone(),
                audit_sink.clone(),
            ))
            .wrap(AuthenticationMiddleware::new(
                data.clone(),
                authentication.clone(),
            ))
            .wrap(Logger::default().exclude("/healthz"))
            .app_data(data.clone())
            .app_data(crd_registry_data.clone())
            .app_data(json_config())
            .service(health_check)
            .configure(endpoints::register_openapi_endpoints)
            .into_utoipa_app()
            .configure(endpoints::register_endpoints)
            .into_app()
    })
    .on_connect(store_client_certificate_info);
    let server = if let Some(tls) = tls {
        let config = match build_tls_server_config(tls) {
            Ok(b) => b,
            Err(err) => {
                tracing::error!("Failed to configure TLS: {err}");
                return;
            }
        };
        match server.listen_rustls_0_23(listener, config) {
            Ok(server) => server,
            Err(err) => {
                tracing::error!("Failed to listen on provided socket with TLS: {err}");
                return;
            }
        }
    } else if allow_insecure_http {
        warn!(
            "API server is running without TLS. This is insecure and not recommended for production use."
        );
        match server.listen(listener) {
            Ok(server) => server,
            Err(err) => {
                tracing::error!("Failed to listen on provided socket: {err}");
                return;
            }
        }
    } else {
        tracing::error!(
            "TLS configuration is missing and allow_insecure_http is false. Refusing to start in insecure mode."
        );
        return;
    };
    if let Err(err) = server.run().await {
        tracing::error!("Failed to run server: {err}");
    }
}

fn json_config() -> JsonConfig {
    JsonConfig::default().error_handler(|err, _req| {
        let message = format!("Invalid JSON body: {err}");
        InternalError::from_response(
            err,
            HttpResponse::BadRequest().json(StatusResponse::bad_request(message, None)),
        )
        .into()
    })
}

#[get("/healthz")]
async fn health_check() -> HttpResponse {
    HttpResponse::Ok().finish()
}

fn build_tls_server_config(tls: TlsConfig) -> Result<ServerConfig, std::io::Error> {
    let cert_file = std::fs::read(&tls.cert_file)
        .map_err(|e| std::io::Error::other(format!("Failed to read TLS certificate chain: {e}")))?;
    let certs = CertificateDer::pem_slice_iter(&cert_file)
        .collect::<Result<Vec<CertificateDer<'static>>, _>>()
        .map_err(|e| {
            std::io::Error::other(format!("Failed to parse TLS certificate chain: {e}"))
        })?;
    if certs.is_empty() {
        return Err(std::io::Error::other(
            "Failed to parse TLS certificate chain: no certificates found",
        ));
    }

    let key_file = std::fs::read(&tls.key_file)
        .map_err(|e| std::io::Error::other(format!("Failed to read TLS private key: {e}")))?;
    let keys = PrivateKeyDer::pem_slice_iter(&key_file)
        .collect::<Result<Vec<PrivateKeyDer<'static>>, _>>()
        .map_err(|e| std::io::Error::other(format!("Failed to parse TLS private key: {e}")))?;
    let [key] = keys.as_slice() else {
        return Err(std::io::Error::other(format!(
            "Failed to parse TLS private key: expected exactly one private key, found {}",
            keys.len()
        )));
    };
    let key = key.clone_key();

    let provider = Arc::new(crypto::ring::default_provider());
    let builder = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| std::io::Error::other(format!("Failed to create TLS configuration: {e}")))?;
    let mut config = if let Some(client_ca_file) = tls.client_cert_file {
        let ca_file = std::fs::read(&client_ca_file)
            .map_err(|e| std::io::Error::other(format!("Failed to read client CA file: {e}")))?;
        let ca_certs = CertificateDer::pem_slice_iter(&ca_file)
            .collect::<Result<Vec<CertificateDer<'static>>, _>>()
            .map_err(|e| std::io::Error::other(format!("Failed to parse client CA file: {e}")))?;
        if ca_certs.is_empty() {
            return Err(std::io::Error::other(
                "Failed to parse client CA file: no certificates found",
            ));
        }
        let mut roots = RootCertStore::empty();
        for cert in ca_certs {
            roots.add(cert).map_err(|e| {
                std::io::Error::other(format!("Failed to add client CA certificate: {e}"))
            })?;
        }
        let verifier = WebPkiClientVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(crypto::ring::default_provider()),
        )
        .build()
        .map_err(|e| std::io::Error::other(format!("Failed to configure client CA: {e}")))?;
        builder
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs, key)
    } else {
        builder.with_no_client_auth().with_single_cert(certs, key)
    }
    .map_err(|e| std::io::Error::other(format!("Failed to configure TLS: {e}")))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

fn store_client_certificate_info(connection: &dyn Any, data: &mut Extensions) {
    let Some(stream) = connection
        .downcast_ref::<actix_tls::accept::rustls_0_23::TlsStream<actix_web::rt::net::TcpStream>>()
    else {
        return;
    };
    if let Some(cert) = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
    {
        match ClientCertificateInfo::from_der(cert.as_ref()) {
            Ok(info) => {
                data.insert(info);
            }
            Err(err) => {
                tracing::warn!("Failed to parse verified client certificate subject: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build_tls_server_config;
    use crate::config::TlsConfig;
    use rcgen::generate_simple_self_signed;
    use std::fs;

    #[test]
    fn tls_configuration_accepts_pem_certificate_and_key() {
        let dir = tempfile::tempdir().expect("temp dir");
        let certified = generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("certificate should be generated");
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        fs::write(&cert_path, certified.cert.pem()).expect("certificate should be written");
        fs::write(&key_path, certified.signing_key.serialize_pem()).expect("key should be written");

        let config = build_tls_server_config(TlsConfig {
            cert_file: cert_path.to_string_lossy().into_owned(),
            key_file: key_path.to_string_lossy().into_owned(),
            client_cert_file: None,
        })
        .expect("TLS configuration should be valid");
        assert_eq!(
            config.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        );
    }
}
