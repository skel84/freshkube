//! What stands in front of the MCP service: the bearer token, the `Origin`
//! and `Host` checks, and the body limit. Every refusal is a bare status, so
//! a probe learns nothing from the body.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio_util::sync::CancellationToken;

use crate::session::Token;

/// A request body over this is refused with 413.
pub(super) const MAX_BODY_BYTES: usize = 64 * 1024;

pub(super) struct Guard {
    pub(super) token: Token,
    /// `127.0.0.1:<port>`, the only `Host` the endpoint answers to.
    pub(super) host: String,
    /// Cancelled when the endpoint is dropped: the token is dead from then
    /// on, even for a connection that was kept open.
    pub(super) ended: CancellationToken,
    /// How long a request's body may take to arrive.
    pub(super) body_timeout: Duration,
}

pub(super) async fn check(
    State(guard): State<Arc<Guard>>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let authorized = !guard.ended.is_cancelled()
        && headers
            .get(header::AUTHORIZATION)
            .is_some_and(|value| guard.token.accepts(value.as_bytes()));
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // A browser page cannot be the agent: it always sends an Origin. And a
    // Host that is not ours is a rebinding attempt.
    let host_ok = headers
        .get(header::HOST)
        .is_some_and(|host| host.as_bytes() == guard.host.as_bytes());
    if headers.contains_key(header::ORIGIN) || !host_ok {
        return StatusCode::FORBIDDEN.into_response();
    }
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|length| length.to_str().ok())
        .and_then(|length| length.parse::<usize>().ok());
    if declared.is_some_and(|length| length > MAX_BODY_BYTES) {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    // A chunked body has no length to declare, so it is read to the limit.
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(
        guard.body_timeout,
        axum::body::to_bytes(body, MAX_BODY_BYTES),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}
