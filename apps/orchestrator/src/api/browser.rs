//! Browser sessions share backend authorization with bearer API clients.

use crate::identity::{Identity, IdentityError};
use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method},
    middleware::Next,
    response::{IntoResponse, Response},
};

pub(super) fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, IdentityError> {
    let mut found = None;
    for header in headers.get_all("cookie") {
        for item in header
            .to_str()
            .map_err(|_| IdentityError::Unauthorized)?
            .split(';')
        {
            if let Some((key, value)) = item.trim().split_once('=')
                && key == name
            {
                if found.is_some() {
                    return Err(IdentityError::Unauthorized);
                }
                found = Some(value);
            }
        }
    }
    found.ok_or(IdentityError::Unauthorized)
}

fn session_name(identity: &Identity) -> &'static str {
    if identity
        .browser_origin
        .as_deref()
        .is_some_and(|origin| origin.starts_with("https://"))
    {
        "__Host-cubebot_session"
    } else {
        "cubebot_session"
    }
}

pub(super) fn credential<'a>(
    identity: &Identity,
    headers: &'a HeaderMap,
) -> Result<&'a str, IdentityError> {
    if let Some(header) = headers.get("authorization") {
        return header
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(IdentityError::Unauthorized);
    }
    identity
        .browser_origin
        .as_ref()
        .ok_or(IdentityError::Unauthorized)?;
    cookie(headers, session_name(identity))
}

pub(super) fn session_cookie(identity: &Identity, token: &str, age: i32) -> Option<String> {
    let origin = identity.browser_origin.as_ref()?;
    let secure = if origin.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    Some(format!(
        "{}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={age}{secure}",
        session_name(identity)
    ))
}

pub(super) fn set_session(
    mut response: Response,
    identity: &Identity,
    token: &str,
    age: i32,
) -> Response {
    if let Some(cookie) = session_cookie(identity, token, age)
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        response.headers_mut().append("set-cookie", value);
    }
    response
}

pub(super) async fn csrf(
    State(identity): State<Identity>,
    request: Request,
    next: Next,
) -> Response {
    let safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let cookie_auth = request.headers().contains_key("cookie")
        && !request.headers().contains_key("authorization");
    if !safe && cookie_auth {
        let origin = request
            .headers()
            .get("origin")
            .and_then(|value| value.to_str().ok());
        if origin.is_none() || origin != identity.browser_origin.as_deref() {
            return IdentityError::Forbidden.into_response();
        }
    }
    next.run(request).await
}
