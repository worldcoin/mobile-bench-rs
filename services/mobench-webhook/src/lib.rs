pub mod config;
pub mod ingest;
mod router;

use anyhow::{Context, Result};
use axum::Router;

pub use router::build_router as app;

pub fn app_for_test() -> Router {
    app()
}

pub async fn serve() -> Result<()> {
    let config = config::Config::from_env()?;
    let listener = tokio::net::TcpListener::bind(config.public_http_addr)
        .await
        .with_context(|| format!("binding {}", config.public_http_addr))?;

    axum::serve(listener, app())
        .await
        .context("serving mobench-webhook")?;

    Ok(())
}
