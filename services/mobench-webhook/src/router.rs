use axum::{
    Router,
    http::StatusCode,
    routing::{get, post},
};

use crate::{AppState, webhook::receive::receive_webhook};

pub fn build_router() -> Router {
    Router::new().route("/healthz", get(healthz))
}

pub fn build_router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/webhook", post(receive_webhook))
        .with_state(state)
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}
