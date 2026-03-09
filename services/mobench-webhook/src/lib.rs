pub mod config;
pub mod db;
pub mod github;
pub mod ingest;
pub mod api;
mod router;
pub mod webhook;
pub mod worker;

use anyhow::{Context, Result};
use axum::Router;
use sqlx::PgPool;

pub use router::build_public_router as app;

#[derive(Clone)]
pub struct AppState {
    pub(crate) config: config::Config,
    pub(crate) repos: db::Repositories,
    pub(crate) github: Option<github::GitHubClients>,
}

impl AppState {
    pub fn new(config: config::Config, pool: PgPool) -> Self {
        Self::with_github(config, pool, None)
    }

    pub fn with_github(
        config: config::Config,
        pool: PgPool,
        github: Option<github::GitHubClients>,
    ) -> Self {
        Self {
            config,
            repos: db::Repositories::new(pool),
            github,
        }
    }

    pub fn for_test(pool: PgPool) -> Self {
        Self::new(config::Config::for_test(), pool)
    }

    pub fn repos(&self) -> &db::Repositories {
        &self.repos
    }
}

pub fn app_for_test() -> Router {
    app()
}

pub fn app_for_test_with_pool(pool: PgPool) -> Router {
    router::build_public_router_with_state(AppState::for_test(pool))
}

pub fn private_app_for_test_with_pool(pool: PgPool) -> Router {
    router::build_private_router_with_state(AppState::for_test(pool))
}

pub async fn serve() -> Result<()> {
    let config = config::Config::from_env()?;
    let public_listener = tokio::net::TcpListener::bind(config.public_http_addr)
        .await
        .with_context(|| format!("binding {}", config.public_http_addr))?;
    let private_listener = tokio::net::TcpListener::bind(config.private_http_addr)
        .await
        .with_context(|| format!("binding {}", config.private_http_addr))?;
    let pool = PgPool::connect(&config.database_url)
        .await
        .context("connecting mobench-webhook database")?;
    db::MIGRATOR.run(&pool).await.context("running migrations")?;
    let auth = github::auth::GitHubAppAuth::new(&config)?;
    let github = github::GitHubClients::new(auth, config.github_api_base_url.clone());
    let state = AppState::with_github(config, pool, Some(github));
    let public_router = router::build_public_router_with_state(state.clone());
    let private_router = router::build_private_router_with_state(state.clone());

    tokio::try_join!(
        async {
            axum::serve(public_listener, public_router)
                .await
                .context("serving public mobench-webhook")
        },
        async {
            axum::serve(private_listener, private_router)
                .await
                .context("serving private mobench-webhook")
        },
        async {
            worker::worker_loop(state)
                .await
                .context("running mobench-webhook worker")
        }
    )?;

    Ok(())
}
