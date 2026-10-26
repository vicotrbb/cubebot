use crate::identity::IdentityError;
use openidconnect::{AsyncHttpClient, HttpRequest, HttpResponse, reqwest};
use std::{fmt, future::Future, pin::Pin, time::Duration};

#[derive(Clone)]
pub(super) struct Http {
    client: reqwest::Client,
    local_http: bool,
}

#[derive(Debug)]
pub(super) struct TransportError;
impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OIDC HTTP transport failed")
    }
}
impl std::error::Error for TransportError {}

impl Http {
    pub fn new(local_http: bool) -> Result<Self, IdentityError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| IdentityError::OidcUnavailable)?;
        Ok(Self { client, local_http })
    }
}

impl<'c> AsyncHttpClient<'c> for Http {
    type Error = TransportError;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send + 'c>>;
    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            super::config::endpoint(&request.uri().to_string(), self.local_http)
                .map_err(|_| TransportError)?;
            let request = request.try_into().map_err(|_| TransportError)?;
            let mut response = self
                .client
                .execute(request)
                .await
                .map_err(|_| TransportError)?;
            let mut result = HttpResponse::default();
            *result.status_mut() = response.status();
            *result.headers_mut() = response.headers().clone();
            while let Some(chunk) = response.chunk().await.map_err(|_| TransportError)? {
                if result.body().len() + chunk.len() > 1_048_576 {
                    return Err(TransportError);
                }
                result.body_mut().extend_from_slice(&chunk);
            }
            Ok(result)
        })
    }
}
