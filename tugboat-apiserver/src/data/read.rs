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

use crate::data::{Table, wants_table_response};
use actix_web::body::BoxBody;
use actix_web::{HttpRequest, HttpResponse, Responder};
use serde::Serialize;

pub(crate) struct ReadResponse<T> {
    data: T,
}

impl<T: Serialize> ReadResponse<T> {
    pub(crate) fn new(data: T) -> Self {
        Self { data }
    }
}

impl<T: Serialize> Responder for ReadResponse<T> {
    type Body = BoxBody;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        if wants_table_response(req) {
            match Table::from_serializable(&self.data) {
                Ok(table) => HttpResponse::Ok().json(table),
                Err(_) => HttpResponse::InternalServerError().finish(),
            }
        } else {
            HttpResponse::Ok().json(&self.data)
        }
    }
}
