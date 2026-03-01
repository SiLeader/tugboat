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
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use openssl::x509::X509;
use utoipa_actix_web::AppExt;

pub mod config;
mod data;
mod endpoints;
mod name_generator;
mod operator;

pub struct ApiServer {
    listen: String,
    operator: ApiOperator,
    tls: Option<TlsConfig>,
}

impl ApiServer {
    fn new(listen: String, operator: ApiOperator, tls: Option<TlsConfig>) -> Self {
        Self {
            listen,
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
                .configure(endpoints::register_openapi_endpoints)
                .into_utoipa_app()
                .configure(endpoints::register_endpoints)
                .into_app()
        });
        if let Some(tls) = self.tls {
            let builder = {
                let mut b = SslAcceptor::mozilla_modern_v5(SslMethod::tls_server())
                    .expect("Failed to create acceptor");
                b.set_private_key_file(tls.key_file, SslFiletype::PEM)
                    .expect("Failed to set key file");
                b.set_certificate_chain_file(tls.cert_file)
                    .expect("Failed to set cert file");
                if let Some(client_ca_file) = tls.client_cert_file {
                    let file =
                        std::fs::read(client_ca_file).expect("Failed to read client CA file");
                    let x509 = X509::from_pem(file.as_slice()).unwrap();
                    b.add_client_ca(x509.as_ref())
                        .expect("Failed to add client CA");
                }
                b
            };
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
}

#[get("/healthz")]
async fn health_check() -> HttpResponse {
    HttpResponse::Ok().finish()
}
