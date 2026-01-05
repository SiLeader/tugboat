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

use actix_web::body::BoxBody;
use actix_web::{HttpRequest, HttpResponse, Responder};
use serde::Serialize;

pub(crate) enum CreateResponse<T> {
    Created(T),
}

impl<T: Serialize> Responder for CreateResponse<T> {
    type Body = BoxBody;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        match self {
            CreateResponse::Created(body) => HttpResponse::Created().json(body),
        }
    }
}
