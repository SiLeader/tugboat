use crate::{Error, TugboatClient};
use async_stream::stream;
use futures::{Stream, StreamExt, TryStreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};
use tokio_util::io::StreamReader;

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
    ) -> Result<impl Stream<Item = Result<WatchEvent<T>, Error>>, Error> {
        let path = format!("{}/{path}", self.base_url);

        let res = self.client.get(path).send().await?;
        let stream = res
            .bytes_stream()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
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
