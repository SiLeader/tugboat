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

use crate::config::TlsConfig;
use crate::operator::ApiOperator;
use actix_web::middleware::Logger;
use actix_web::web::Data;
use actix_web::{App, HttpResponse, HttpServer, get};
use rustls::pki_types::CertificateDer;
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use utoipa_actix_web::AppExt;

pub mod config;
mod data;
mod endpoints;
mod name_generator;
mod operator;

pub struct ApiServer {
    listen: String,
    mount: String,
    operator: ApiOperator,
    tls: Option<TlsConfig>,
}

impl ApiServer {
    fn new(listen: String, mount: String, operator: ApiOperator, tls: Option<TlsConfig>) -> Self {
        Self {
            listen,
            mount,
            operator,
            tls,
        }
    }

    pub async fn run(self) {
        let data = Data::new(self.operator);
        let server = HttpServer::new(move || {
            App::new()
                .wrap(Logger::default().exclude("/healthz"))
                .app_data(data.clone())
                .service(health_check)
                .into_utoipa_app()
                .service(
                    utoipa_actix_web::scope(actix_web::web::scope(&self.mount))
                        .configure(endpoints::register_endpoints),
                )
                .into_app()
        });
        if let Some(tls) = self.tls {
            let cert_file = File::open(tls.cert_file).expect("Failed to open cert file");
            let mut cert_file = BufReader::new(cert_file);

            let certs = rustls_pemfile::certs(&mut cert_file)
                .collect::<Result<Vec<_>, _>>()
                .expect("Failed to parse cert file")
                .into_iter()
                .map(CertificateDer::from)
                .collect();

            let key_file = File::open(tls.key_file).expect("Failed to open key file");
            let mut key_file = BufReader::new(key_file);
            let key = rustls_pemfile::private_key(&mut key_file)
                .expect("Failed to parse key file")
                .expect("Private key cannot be loaded");

            let builder = ServerConfig::builder();
            let builder = if let Some(client_cert_file) = tls.client_cert_file {
                let cert_file =
                    File::open(client_cert_file).expect("Failed to open client cert file");
                let mut cert_file = BufReader::new(cert_file);

                let certs = rustls_pemfile::certs(&mut cert_file)
                    .collect::<Result<Vec<_>, _>>()
                    .expect("Failed to parse client cert file")
                    .into_iter()
                    .map(CertificateDer::from)
                    .collect::<Vec<_>>();

                let mut store = RootCertStore::empty();
                for cert in certs {
                    store.add(cert).expect("Failed to add client cert to store");
                }
                builder.with_client_cert_verifier(
                    WebPkiClientVerifier::builder(Arc::new(store))
                        .build()
                        .expect("Failed to build client cert verifier"),
                )
            } else {
                builder.with_no_client_auth()
            };
            let config = builder
                .with_single_cert(certs, key)
                .expect("Failed to build TLS config");
            server
                .bind_rustls_0_23(self.listen, config)
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
}

#[get("/healthz")]
async fn health_check() -> HttpResponse {
    HttpResponse::Ok().finish()
}
