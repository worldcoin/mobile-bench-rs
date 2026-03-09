use axum::{
    Router,
    http::StatusCode,
    routing::{get, post},
};

use crate::{AppState, api, webhook::receive::receive_webhook};

pub fn build_public_router() -> Router {
    Router::new().route("/healthz", get(healthz))
}

pub fn build_public_router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/webhook", post(receive_webhook))
        .with_state(state)
}

pub fn build_private_router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/api/healthz", get(healthz))
        .route("/api/workflow-runs", get(api::list_workflow_runs))
        .route("/api/workflow-runs/{workflow_run_id}", get(api::get_workflow_run))
        .route("/api/platform-runs/{platform_run_id}", get(api::get_platform_run))
        .route("/api/trends", get(api::get_trends))
        .route("/api/compare", get(api::compare_platform_run))
        .with_state(state)
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}
