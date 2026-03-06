pub mod config;
pub mod db;
pub mod ingest;
mod router;
pub mod webhook;

use anyhow::{Context, Result};
use axum::Router;
use sqlx::PgPool;

pub use router::build_router as app;

#[derive(Clone)]
pub struct AppState {
    pub(crate) config: config::Config,
    pub(crate) repos: db::Repositories,
}

impl AppState {
    pub fn new(config: config::Config, pool: PgPool) -> Self {
        Self {
            config,
            repos: db::Repositories::new(pool),
        }
    }

    pub fn for_test(pool: PgPool) -> Self {
        Self::new(config::Config::for_test(), pool)
    }
}

pub fn app_for_test() -> Router {
    app()
}

pub fn app_for_test_with_pool(pool: PgPool) -> Router {
    router::build_router_with_state(AppState::for_test(pool))
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
