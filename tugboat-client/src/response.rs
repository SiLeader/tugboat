use crate::TugboatClient;
use crate::error::{ApiStatus, Error};
use reqwest::Response;
use serde::Deserialize;
use serde::de::DeserializeOwned;

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
            Self::parse_impl(response).await
        }
    }

    pub(crate) async fn parse_response_list<T: DeserializeOwned>(
        response: Response,
    ) -> Result<Vec<T>, Error> {
        let list: ListResponse<T> = Self::parse_impl(response).await?;
        Ok(list.items)
    }

    async fn parse_impl<T: DeserializeOwned>(response: Response) -> Result<T, Error> {
        if response.status().is_success() {
            Ok(response.json().await?)
        } else {
            let status: ApiStatus = response.json().await?;
            Err(Error::Api(status))
        }
    }
}
