//! Machine credential boundary. It never consults human cookies or human session authority.
use crate::agents::{
    kubernetes::Error,
    runtime::{RuntimeAccess, RuntimeActor, RuntimeIdentity, RuntimeSession},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bootstrap {
    token: String,
    instance: Uuid,
}

pub(super) fn router(identity: RuntimeIdentity) -> Router {
    Router::new()
        .route("/api/runtime/bootstrap", post(bootstrap))
        .route("/api/runtime/identity", get(inspect))
        .layer(DefaultBodyLimit::max(20 * 1024))
        .layer(axum::middleware::map_response(no_store))
        .with_state(identity)
}
async fn bootstrap(
    State(identity): State<RuntimeIdentity>,
    input: Result<Json<Bootstrap>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<RuntimeSession>, Failure> {
    let Json(input) = input.map_err(|error| Failure(error.status()))?;
    let session: RuntimeSession = identity
        .bootstrap(&input.token, input.instance)
        .await
        .map_err(Failure::from)?;
    Ok(Json(session))
}
async fn inspect(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
) -> Result<Json<RuntimeActor>, Failure> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(Failure(StatusCode::UNAUTHORIZED))?;
    let actor: RuntimeActor = identity
        .authenticate(token, RuntimeAccess::Active)
        .await
        .map_err(Failure::from)?;
    Ok(Json(actor))
}
struct Failure(StatusCode);
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self(match error {
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Conflict => StatusCode::CONFLICT,
            Error::Busy => StatusCode::TOO_MANY_REQUESTS,
            Error::InvalidConfiguration => StatusCode::BAD_REQUEST,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(serde_json::json!({"error":"runtime_identity_unavailable"})),
        )
            .into_response()
    }
}

async fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        response.headers_mut().insert(
            header::RETRY_AFTER,
            axum::http::HeaderValue::from_static("1"),
        );
    }
    response
}
