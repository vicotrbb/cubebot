//! Machine credential boundary. It never consults human cookies or human session authority.
use crate::agents::{
    kubernetes::Error,
    runtime::{RuntimeAccess, RuntimeIdentity},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State, ws::WebSocketUpgrade},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_protocol::runtime::{Bootstrap, RuntimeActor, RuntimeSession};

pub(super) fn router(identity: RuntimeIdentity) -> Router {
    Router::new()
        .route("/api/runtime/bootstrap", post(bootstrap))
        .route("/api/runtime/identity", get(inspect))
        .route("/api/runtime/channel", get(channel))
        .layer(DefaultBodyLimit::max(20 * 1024))
        .layer(axum::middleware::map_response(no_store))
        .with_state(identity)
}
async fn bootstrap(
    State(identity): State<RuntimeIdentity>,
    input: Result<Json<Bootstrap>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<RuntimeSession>, Failure> {
    let Json(input) = input.map_err(|error| Failure(error.status()))?;
    let session = identity
        .bootstrap(&input.token, input.instance)
        .await
        .map_err(Failure::from)?;
    Ok(Json(session.into()))
}
async fn inspect(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
) -> Result<Json<RuntimeActor>, Failure> {
    let token = bearer(&headers)?;
    let actor = identity
        .authenticate(token, RuntimeAccess::Active)
        .await
        .map_err(Failure::from)?;
    Ok(Json(actor.into()))
}
/// Authenticates before upgrading; the channel rechecks the credential while it runs.
async fn channel(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Failure> {
    let token = bearer(&headers)?.to_owned();
    identity
        .authenticate(&token, RuntimeAccess::Recovery)
        .await
        .map_err(Failure::from)?;
    Ok(upgrade
        .max_message_size(2 * cubebot_protocol::runtime::EVENT_BYTES)
        .on_upgrade(move |socket| identity.serve(token, socket)))
}
fn bearer(headers: &HeaderMap) -> Result<&str, Failure> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(Failure(StatusCode::UNAUTHORIZED))
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
