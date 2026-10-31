use crate::identity::{IdentityError, Source};
use openidconnect::{AsyncHttpClient, HttpRequest, HttpResponse, reqwest};
use std::{fmt, future::Future, pin::Pin, time::Duration};

#[derive(Clone)]
pub(super) struct Http {
    client: reqwest::Client,
    local_http: bool,
}

/// `None` when the response body exceeded its limit.
#[derive(Debug)]
pub(super) struct TransportError(Option<Source>);
impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OIDC HTTP transport failed")
    }
}
impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.as_deref().map(|error| error as _)
    }
}
// Request URLs can carry authorization parameters, so causes never keep them.
fn transport(error: reqwest::Error) -> TransportError {
    TransportError(Some(error.without_url().into()))
}

impl Http {
    pub fn new(local_http: bool) -> Result<Self, IdentityError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|error| IdentityError::OidcUnavailable(error.into()))?;
        Ok(Self { client, local_http })
    }
}

impl<'c> AsyncHttpClient<'c> for Http {
    type Error = TransportError;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send + 'c>>;
    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            super::config::endpoint(&request.uri().to_string(), self.local_http)
                .map_err(|error| TransportError(Some(error.into())))?;
            let request = request.try_into().map_err(transport)?;
            let mut response = self.client.execute(request).await.map_err(transport)?;
            let mut result = HttpResponse::default();
            *result.status_mut() = response.status();
            *result.headers_mut() = response.headers().clone();
            while let Some(chunk) = response.chunk().await.map_err(transport)? {
                if result.body().len() + chunk.len() > 1_048_576 {
                    return Err(TransportError(None));
                }
                result.body_mut().extend_from_slice(&chunk);
            }
            Ok(result)
        })
    }
}
