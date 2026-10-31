use super::Error;
use serde::de::DeserializeOwned;

pub(super) async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, Error> {
    decode_bounded(response, 20 * 1024).await
}

pub(super) async fn decode_bounded<T: DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
) -> Result<T, Error> {
    serde_json::from_slice(&bytes(response, limit).await?).map_err(Error::protocol)
}

/// A successful response's raw body of at most `limit` bytes.
pub(super) async fn bytes(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, Error> {
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => Error::Rejected,
            _ => Error::Unavailable(None),
        });
    }
    read(response, limit).await
}

/// A refusal's small JSON body.
pub(super) async fn decode_conflict<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, Error> {
    body(response, 4 * 1024).await
}

async fn body<T: DeserializeOwned>(response: reqwest::Response, limit: usize) -> Result<T, Error> {
    serde_json::from_slice(&read(response, limit).await?).map_err(Error::protocol)
}

async fn read(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(super::request_failed)? {
        if bytes.len() + chunk.len() > limit {
            return Err(Error::Protocol(None));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
