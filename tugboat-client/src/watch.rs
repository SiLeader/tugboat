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

use crate::{Error, TugboatClient};
use async_stream::stream;
use futures::{Stream, StreamExt, TryStreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};
use tokio_util::io::StreamReader;

#[derive(Clone, Debug, Default)]
pub struct WatchParams {
    pub label_selector: Option<String>,
    pub field_selector: Option<String>,
    pub resource_version: Option<String>,
}

impl WatchParams {
    pub fn fields(mut self, field_selector: impl ToString) -> Self {
        self.field_selector = Some(field_selector.to_string());
        self
    }

    pub fn labels(mut self, label_selector: impl ToString) -> Self {
        self.label_selector = Some(label_selector.to_string());
        self
    }

    pub fn resource_version(mut self, resource_version: impl ToString) -> Self {
        self.resource_version = Some(resource_version.to_string());
        self
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "object", rename_all = "UPPERCASE")]
pub enum WatchEvent<T> {
    Added(T),
    Modified(T),
    Deleted(T),
}

impl TugboatClient {
    pub(crate) async fn watch_impl<T: DeserializeOwned>(
        &self,
        path: String,
        params: &WatchParams,
    ) -> Result<impl Stream<Item = Result<WatchEvent<T>, Error>>, Error> {
        let mut url = self.build_url(&path);
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("watch", "true");
            if let Some(l) = &params.label_selector {
                pairs.append_pair("labelSelector", l);
            }
            if let Some(f) = &params.field_selector {
                pairs.append_pair("fieldSelector", f);
            }
            if let Some(rv) = &params.resource_version {
                pairs.append_pair("resourceVersion", rv);
            }
        }

        let res = self.client.get(url).send().await?;
        let stream = res.bytes_stream().map_err(std::io::Error::other);
        let reader = StreamReader::new(stream);
        let mut lines = FramedRead::new(reader, LinesCodec::new());

        Ok(stream! {
            while let Some(line) = lines.next().await {
                if let Some(line) = parse::<T>(line).await {
                    yield line;
                }
            }
        })
    }
}

async fn parse<T: DeserializeOwned>(
    result: Result<String, LinesCodecError>,
) -> Option<Result<WatchEvent<T>, Error>> {
    match result {
        Ok(line) => {
            if line.trim().is_empty() {
                None
            } else {
                Some(serde_json::from_str::<WatchEvent<T>>(&line).map_err(Error::Deserialize))
            }
        }
        Err(e) => Some(Err(Error::LineParse(e))),
    }
}
