use axum::{Router, http::StatusCode, routing::get};

pub fn build_router() -> Router {
    Router::new().route("/healthz", get(healthz))
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}
