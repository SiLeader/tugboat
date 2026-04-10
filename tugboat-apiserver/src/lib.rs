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

use crate::auth::middleware::{
    AuthenticationMiddleware, AuthorizationMiddleware, ClientCertificateInfo,
};
use crate::config::{AuthenticationConfig, AuthorizationConfig, TlsConfig};
use crate::data::StatusResponse;
use crate::operator::ApiOperator;
use actix_web::error::InternalError;
use actix_web::middleware::Logger;
use actix_web::web::{Data, JsonConfig};
use actix_web::{App, HttpResponse, HttpServer, dev::Extensions, get};
use openssl::ssl::{SslAcceptor, SslAcceptorBuilder, SslFiletype, SslMethod, SslVerifyMode};
use openssl::x509::X509;
use std::any::Any;
use std::net::TcpListener;
use utoipa_actix_web::AppExt;

pub mod auth;
pub mod config;
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
}

impl ApiServer {
    fn new(
        listen: String,
        operator: ApiOperator,
        tls: Option<TlsConfig>,
        authentication: AuthenticationConfig,
        authorization: AuthorizationConfig,
    ) -> Self {
        Self {
            listen,
            operator,
            tls,
            authentication,
            authorization,
        }
    }

    pub async fn run(self) {
        let data = Data::new(self.operator);
        let authentication = self.authentication.clone();
        let authorization = self.authorization.clone();
        let server = HttpServer::new(move || {
            App::new()
                .wrap(Logger::default().exclude("/healthz"))
                .wrap(AuthorizationMiddleware::new(
                    data.clone(),
                    authorization.clone(),
                ))
                .wrap(AuthenticationMiddleware::new(
                    data.clone(),
                    authentication.clone(),
                ))
                .app_data(data.clone())
                .app_data(json_config())
                .service(health_check)
                .configure(endpoints::register_openapi_endpoints)
                .into_utoipa_app()
                .configure(endpoints::register_endpoints)
                .into_app()
        })
        .on_connect(store_client_certificate_info);
        if let Some(tls) = self.tls {
            let builder = build_tls_acceptor(tls);
            server
                .bind_openssl(self.listen, builder)
                .expect("Failed to bind server")
                .run()
                .await
                .expect("Failed to run server");
        } else {
            server
                .bind(self.listen)
                .expect("Failed to bind server")
                .run()
                .await
                .expect("Failed to run server");
        }
    }

    pub async fn run_with_listener(self, listener: TcpListener) {
        let data = Data::new(self.operator);
        let authentication = self.authentication.clone();
        let authorization = self.authorization.clone();
        HttpServer::new(move || {
            App::new()
                .wrap(Logger::default().exclude("/healthz"))
                .wrap(AuthorizationMiddleware::new(
                    data.clone(),
                    authorization.clone(),
                ))
                .wrap(AuthenticationMiddleware::new(
                    data.clone(),
                    authentication.clone(),
                ))
                .app_data(data.clone())
                .app_data(json_config())
                .service(health_check)
                .configure(endpoints::register_openapi_endpoints)
                .into_utoipa_app()
                .configure(endpoints::register_endpoints)
                .into_app()
        })
        .on_connect(store_client_certificate_info)
        .listen(listener)
        .expect("Failed to listen on provided socket")
        .run()
        .await
        .expect("Failed to run server");
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

fn build_tls_acceptor(tls: TlsConfig) -> SslAcceptorBuilder {
    let mut builder =
        SslAcceptor::mozilla_modern_v5(SslMethod::tls_server()).expect("Failed to create acceptor");
    builder
        .set_private_key_file(tls.key_file, SslFiletype::PEM)
        .expect("Failed to set key file");
    builder
        .set_certificate_chain_file(tls.cert_file)
        .expect("Failed to set cert file");
    if let Some(client_ca_file) = tls.client_cert_file {
        configure_client_certificate_auth(&mut builder, &client_ca_file);
    }
    builder
}

fn configure_client_certificate_auth(builder: &mut SslAcceptorBuilder, client_ca_file: &str) {
    let file = std::fs::read(client_ca_file).expect("Failed to read client CA file");
    let certs = X509::stack_from_pem(file.as_slice()).expect("Failed to parse client CA file");
    builder
        .set_ca_file(client_ca_file)
        .expect("Failed to set client CA file");
    for cert in certs {
        builder
            .add_client_ca(cert.as_ref())
            .expect("Failed to add client CA");
    }
    builder.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
}

fn store_client_certificate_info(connection: &dyn Any, data: &mut Extensions) {
    let Some(stream) = connection
        .downcast_ref::<actix_tls::accept::openssl::TlsStream<actix_web::rt::net::TcpStream>>()
    else {
        return;
    };
    if let Some(cert) = stream.ssl().peer_certificate() {
        data.insert(ClientCertificateInfo::from_x509(&cert));
    }
}

#[cfg(test)]
mod tests {
    use super::configure_client_certificate_auth;
    use openssl::asn1::Asn1Time;
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::ssl::{SslAcceptor, SslMethod, SslVerifyMode};
    use openssl::x509::{X509, X509NameBuilder};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn client_ca_configuration_enables_peer_verification() {
        let mut builder = SslAcceptor::mozilla_modern_v5(SslMethod::tls_server())
            .expect("acceptor should be created");
        let path = write_temp_cert_file(generate_test_cert_pem());

        configure_client_certificate_auth(&mut builder, path.to_str().expect("utf-8 path"));

        let verify_mode = builder.build().context().verify_mode();
        assert!(verify_mode.contains(SslVerifyMode::PEER));
        assert!(verify_mode.contains(SslVerifyMode::FAIL_IF_NO_PEER_CERT));

        let _ = fs::remove_file(path);
    }

    fn generate_test_cert_pem() -> Vec<u8> {
        let rsa = Rsa::generate(2048).expect("rsa should be generated");
        let pkey = PKey::from_rsa(rsa).expect("pkey should be generated");
        let mut name = X509NameBuilder::new().expect("name builder should be created");
        name.append_entry_by_text("CN", "tugboat-test-ca")
            .expect("common name should be set");
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
        cert.build().to_pem().expect("certificate should serialize")
    }

    fn write_temp_cert_file(contents: Vec<u8>) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tugboat-test-ca-{unique}.pem"));
        fs::write(&path, contents).expect("temp cert file should be written");
        path
    }
}
