//! Machine credential boundary. It never consults human cookies or human session authority.
use crate::agents::{
    kubernetes::Error,
    runtime::{RuntimeAccess, RuntimeIdentity},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_protocol::runtime::{
    Bootstrap, DrainAcknowledgement, DrainRequest, JournalReceipt, JournalRecord, RuntimeActor,
    RuntimeControl, RuntimeSession,
};

pub(super) fn router(identity: RuntimeIdentity) -> Router {
    Router::new()
        .route("/api/runtime/bootstrap", post(bootstrap))
        .route("/api/runtime/identity", get(inspect))
        .route("/api/runtime/control", get(control))
        .route("/api/runtime/text/claim", post(claim_text))
        .route("/api/runtime/terminal", post(terminal))
        .route("/api/runtime/terminal/renew", post(renew_terminal))
        .route("/api/runtime/terminal/pending", get(pending_terminal))
        .route("/api/runtime/drained", post(drained))
        .layer(DefaultBodyLimit::max(20 * 1024))
        .merge(
            Router::new()
                .route("/api/runtime/journal", post(journal))
                .layer(DefaultBodyLimit::max(272 * 1024)),
        )
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
async fn control(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
) -> Result<Json<RuntimeControl>, Failure> {
    Ok(Json(
        identity
            .control(bearer(&headers)?)
            .await
            .map_err(Failure::from)?
            .into(),
    ))
}
async fn drained(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    input: Result<Json<DrainAcknowledgement>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<DrainRequest>, Failure> {
    let Json(input) = input.map_err(|error| Failure(error.status()))?;
    Ok(Json(
        identity
            .acknowledge_drain(bearer(&headers)?, input.request_id)
            .await
            .map_err(Failure::from)?
            .into(),
    ))
}
async fn journal(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    input: Result<Json<JournalRecord>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<JournalReceipt>, Failure> {
    let Json(record) = input.map_err(|error| Failure(error.status()))?;
    identity
        .retain_journal(bearer(&headers)?, &record)
        .await
        .map(Json)
        .map_err(Failure::from)
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

async fn claim_text(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
) -> Result<Json<Option<cubebot_protocol::chat::InferenceGrant>>, Failure> {
    identity
        .claim_text(bearer(&headers)?)
        .await
        .map(Json)
        .map_err(Failure::from)
}

async fn terminal(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Json(input): Json<cubebot_protocol::terminal::Authorization>,
) -> Result<Json<cubebot_protocol::terminal::Permit>, Failure> {
    identity
        .authorize_terminal(bearer(&headers)?, &input)
        .await
        .map(Json)
        .map_err(Failure::from)
}

async fn pending_terminal(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
) -> Result<Json<Vec<cubebot_protocol::terminal::Authorization>>, Failure> {
    identity
        .pending_terminal(bearer(&headers)?)
        .await
        .map(Json)
        .map_err(Failure::from)
}

async fn renew_terminal(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Json(input): Json<cubebot_protocol::terminal::Renewal>,
) -> Result<Json<cubebot_protocol::terminal::Permit>, Failure> {
    identity
        .renew_terminal(bearer(&headers)?, &input)
        .await
        .map(Json)
        .map_err(Failure::from)
}
