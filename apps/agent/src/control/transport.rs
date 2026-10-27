use super::Error;
use serde::de::DeserializeOwned;

pub(super) async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, Error> {
    decode_bounded(response, 20 * 1024).await
}

pub(super) async fn decode_bounded<T: DeserializeOwned>(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<T, Error> {
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => Error::Rejected,
            _ => Error::Unavailable,
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unavailable)? {
        if bytes.len() + chunk.len() > limit {
            return Err(Error::Protocol);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)
}
