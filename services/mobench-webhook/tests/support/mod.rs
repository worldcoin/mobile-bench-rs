#![allow(dead_code)]

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use sqlx::PgPool;
use tokio::sync::Mutex;

#[derive(Clone)]
struct MockGitHubState {
    workflow_dispatches: Arc<Mutex<Vec<serde_json::Value>>>,
    pull_requests: Arc<Mutex<HashMap<String, serde_json::Value>>>,
}

pub struct Harness {
    state: mobench_webhook::AppState,
    workflow_dispatches: Arc<Mutex<Vec<serde_json::Value>>>,
    pull_requests: Arc<Mutex<HashMap<String, serde_json::Value>>>,
}

impl Harness {
    pub async fn new(pool: PgPool) -> Result<Self> {
        let workflow_dispatches = Arc::new(Mutex::new(Vec::new()));
        let pull_requests = Arc::new(Mutex::new(HashMap::new()));
        let server_state = MockGitHubState {
            workflow_dispatches: workflow_dispatches.clone(),
            pull_requests: pull_requests.clone(),
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
            .route(
                "/repos/{owner}/{repo}/pulls/{number}",
                get(fetch_pull_request),
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
            pull_requests,
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
        self.maybe_seed_pull_request_details(&payload).await?;

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

    async fn maybe_seed_pull_request_details(&self, payload: &serde_json::Value) -> Result<()> {
        let repo_full_name = match payload
            .get("repository")
            .and_then(|repo| repo.get("full_name"))
            .and_then(serde_json::Value::as_str)
        {
            Some(full_name) => full_name,
            None => return Ok(()),
        };
        let issue_number = match payload
            .get("issue")
            .and_then(|issue| issue.get("pull_request"))
            .and_then(serde_json::Value::as_object)
            .and_then(|_| payload.get("issue"))
            .and_then(|issue| issue.get("number"))
            .and_then(serde_json::Value::as_i64)
        {
            Some(number) => number,
            None => return Ok(()),
        };
        let details_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(format!("pull_request_details_{issue_number}.json"));
        if !details_fixture.exists() {
            return Ok(());
        }

        let raw = std::fs::read_to_string(&details_fixture).with_context(|| {
            format!("reading pull request fixture {}", details_fixture.display())
        })?;
        let details: serde_json::Value = serde_json::from_str(&raw).with_context(|| {
            format!("parsing pull request fixture {}", details_fixture.display())
        })?;

        self.pull_requests
            .lock()
            .await
            .insert(format!("{repo_full_name}#{issue_number}"), details);

        Ok(())
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

async fn fetch_pull_request(
    Path((owner, repo, number)): Path<(String, String, i32)>,
    State(state): State<MockGitHubState>,
) -> Json<serde_json::Value> {
    let key = format!("{owner}/{repo}#{number}");
    let payload = state
        .pull_requests
        .lock()
        .await
        .get(&key)
        .cloned()
        .unwrap_or_else(|| {
            serde_json::json!({
                "state": "open",
                "head": {
                    "ref": "feature/bench-pr",
                    "sha": "abc123def456",
                    "repo": {
                        "full_name": format!("{owner}/{repo}")
                    }
                },
                "base": {
                    "repo": {
                        "full_name": format!("{owner}/{repo}")
                    }
                }
            })
        });

    Json(payload)
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
