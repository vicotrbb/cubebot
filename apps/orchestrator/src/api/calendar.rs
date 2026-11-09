//! The person's own calendar: their calendar connections' events and their agents' routines.
//! Routines are written through the routine endpoints.
use crate::{
    calendar::{Calendar, CalendarRange, ChangeEvent, Item, NewEvent, RemoveEvent, Window},
    connections::ConnectError,
    identity::{Identity, IdentityError},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};

#[derive(Clone)]
struct Api {
    identity: Identity,
    calendar: Calendar,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(range, create, update, delete))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity, calendar: Calendar) -> Router {
    Router::new()
        .route("/api/calendar", get(range))
        .route(
            "/api/calendar/events",
            post(create).put(update).delete(delete),
        )
        .layer(DefaultBodyLimit::max(32 * 1024))
        .with_state(Api { identity, calendar })
}

/// The range's events and routine runs, in order of start; 400 for a range over 62 days. A
/// connection that fails shows its problem on its source.
#[utoipa::path(get, path = "/api/calendar", operation_id = "calendar_range", params(Window), responses((status = 200, body = CalendarRange), (status = 400)))]
async fn range(
    State(api): State<Api>,
    headers: HeaderMap,
    Query(window): Query<Window>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let range = api.calendar.range(&actor, &window).await?;
    Ok(([("cache-control", "no-store")], Json(range)))
}

/// 404 for a connection that is not yours, 403 when its level does not allow writing, 422 with
/// the vendor's reason when it refuses.
#[utoipa::path(post, path = "/api/calendar/events", operation_id = "create_calendar_event", request_body = NewEvent, responses((status = 200, body = Item), (status = 400), (status = 403), (status = 404), (status = 422)))]
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<NewEvent>,
) -> Result<Json<Item>, ConnectError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(Json(api.calendar.create_event(&actor, &input).await?))
}

/// Changes the event, or with scope `all` the series its occurrence belongs to; read the range
/// again for the result.
#[utoipa::path(put, path = "/api/calendar/events", operation_id = "update_calendar_event", request_body = ChangeEvent, responses((status = 204), (status = 400), (status = 403), (status = 404), (status = 422)))]
async fn update(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<ChangeEvent>,
) -> Result<StatusCode, ConnectError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.calendar.update_event(&actor, &input).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/calendar/events", operation_id = "delete_calendar_event", request_body = RemoveEvent, responses((status = 204), (status = 403), (status = 404), (status = 422)))]
async fn delete(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<RemoveEvent>,
) -> Result<StatusCode, ConnectError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.calendar.delete_event(&actor, &input).await?;
    Ok(StatusCode::NO_CONTENT)
}
