use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::{
    AppState,
    webhook::verify::{VerifyError, verify_signature},
};

pub async fn receive_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ReceiveError> {
    verify_signature(&state.config.github_webhook_secret, &headers, body.as_ref())
        .map_err(ReceiveError::InvalidSignature)?;

    let delivery_id = required_header(&headers, "X-GitHub-Delivery")?;
    let event = required_header(&headers, "X-GitHub-Event")?;
    let payload: Value = serde_json::from_slice(&body).map_err(ReceiveError::InvalidPayload)?;
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .map(str::to_owned);

    state
        .repos
        .deliveries
        .insert_pending_if_new(delivery_id, event, action.as_deref(), payload)
        .await
        .map_err(ReceiveError::Database)?;

    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug)]
pub enum ReceiveError {
    MissingHeader(&'static str),
    InvalidSignature(VerifyError),
    InvalidPayload(serde_json::Error),
    Database(anyhow::Error),
}

impl IntoResponse for ReceiveError {
    fn into_response(self) -> Response {
        match self {
            Self::MissingHeader(_) | Self::InvalidPayload(_) => {
                StatusCode::BAD_REQUEST.into_response()
            }
            Self::InvalidSignature(_) => StatusCode::UNAUTHORIZED.into_response(),
            Self::Database(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

fn required_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<&'a str, ReceiveError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or(ReceiveError::MissingHeader(name))
}
