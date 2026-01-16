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

use crate::TugboatClient;
use crate::error::{ApiStatus, Error};
use reqwest::Response;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tracing::debug;

#[derive(Deserialize)]
struct ListResponse<T> {
    items: Vec<T>,
}

impl TugboatClient {
    pub(crate) async fn parse_response<T: DeserializeOwned>(
        response: Response,
    ) -> Result<T, Error> {
        Self::parse_impl(response).await
    }

    pub(crate) async fn parse_response_opt<T: DeserializeOwned>(
        response: Response,
    ) -> Result<Option<T>, Error> {
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            Ok(None)
        } else {
            let data = Self::parse_impl(response).await?;
            Ok(Some(data))
        }
    }

    pub(crate) async fn parse_response_list<T: DeserializeOwned>(
        response: Response,
    ) -> Result<Vec<T>, Error> {
        let list: ListResponse<T> = Self::parse_impl(response).await?;
        Ok(list.items)
    }

    async fn parse_impl<T: DeserializeOwned>(response: Response) -> Result<T, Error> {
        let status = response.status();
        let text = response.text().await?;
        debug!("Response from api: {text}");
        if status.is_success() {
            Ok(serde_json::from_str(&text)?)
        } else {
            let status: ApiStatus = serde_json::from_str(&text)?;
            Err(Error::Api(status))
        }
    }
}
