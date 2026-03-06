#![allow(dead_code)]

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use axum::{Json, Router, extract::State, routing::post};
use sqlx::PgPool;
use tokio::sync::Mutex;

#[derive(Clone)]
struct MockGitHubState {
    workflow_dispatches: Arc<Mutex<Vec<serde_json::Value>>>,
}

pub struct Harness {
    state: mobench_webhook::AppState,
    workflow_dispatches: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl Harness {
    pub async fn new(pool: PgPool) -> Result<Self> {
        let workflow_dispatches = Arc::new(Mutex::new(Vec::new()));
        let server_state = MockGitHubState {
            workflow_dispatches: workflow_dispatches.clone(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .context("binding mock GitHub server")?;
        let addr = listener
            .local_addr()
            .context("reading mock GitHub address")?;
        let app = Router::new()
            .route(
                "/app/installations/{installation_id}/access_tokens",
                post(|| async {
                    Json(serde_json::json!({
                        "token": "test-installation-token",
                        "expires_at": "2099-01-01T00:00:00Z"
                    }))
                }),
            )
            .route(
                "/repos/{owner}/{repo}/actions/workflows/{workflow_id}/dispatches",
                post(record_dispatch),
            )
            .with_state(server_state);
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock GitHub server");
        });

        let api_base_url = format!("http://{addr}");
        let auth = mobench_webhook::github::auth::GitHubAppAuth::for_test(api_base_url.clone());
        let github = mobench_webhook::github::GitHubClients::new(auth, api_base_url);
        let state = mobench_webhook::AppState::with_github(
            mobench_webhook::config::Config::for_test(),
            pool,
            Some(github),
        );

        Ok(Self {
            state,
            workflow_dispatches,
        })
    }

    pub async fn enqueue_fixture(&self, fixture_name: &str) -> Result<()> {
        let raw = std::fs::read_to_string(fixture_path(fixture_name))
            .with_context(|| format!("reading fixture {fixture_name}"))?;
        let payload: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("parsing fixture {fixture_name}"))?;
        let event = fixture_event_name(fixture_name);
        let action = payload
            .get("action")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);

        self.state
            .repos()
            .deliveries
            .insert_pending(fixture_name, event, action.as_deref(), payload)
            .await?;

        Ok(())
    }

    pub async fn run_one_delivery(&self) -> Result<bool> {
        mobench_webhook::worker::run_once(&self.state).await
    }

    pub async fn list_dispatches(
        &self,
    ) -> Result<Vec<mobench_webhook::db::models::BenchmarkDispatch>> {
        self.state.repos().dispatches.list_all().await
    }

    pub async fn dispatched_workflows(&self) -> Vec<serde_json::Value> {
        self.workflow_dispatches.lock().await.clone()
    }
}

pub fn app_state(pool: PgPool) -> mobench_webhook::AppState {
    mobench_webhook::AppState::for_test(pool)
}

pub fn repos(pool: PgPool) -> mobench_webhook::db::Repositories {
    mobench_webhook::db::Repositories::new(pool)
}

pub async fn seed_labeled_pull_request_delivery(
    pool: PgPool,
    delivery_id: &str,
) -> Result<mobench_webhook::db::models::DeliveryRecord> {
    repos(pool)
        .deliveries
        .insert_fixture(
            delivery_id,
            "pull_request",
            Some("labeled"),
            r#"{"action":"labeled"}"#,
        )
        .await
}

async fn record_dispatch(
    State(state): State<MockGitHubState>,
    Json(payload): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    state.workflow_dispatches.lock().await.push(payload);

    Json(serde_json::json!({}))
}

fn fixture_event_name(fixture_name: &str) -> &'static str {
    if fixture_name.starts_with("pull_request_") {
        "pull_request"
    } else if fixture_name.starts_with("issue_comment_") {
        "issue_comment"
    } else if fixture_name.starts_with("workflow_run_") {
        "workflow_run"
    } else if fixture_name.starts_with("check_run_") {
        "check_run"
    } else {
        "unknown"
    }
}

fn fixture_path(fixture_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(fixture_name)
}
