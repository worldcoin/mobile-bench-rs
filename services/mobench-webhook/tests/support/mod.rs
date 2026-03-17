#![allow(dead_code)]

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, patch, post},
};
use sqlx::PgPool;
use tokio::sync::Mutex;
use zip::{CompressionMethod, ZipWriter, write::FileOptions};

#[derive(Clone)]
struct MockGitHubState {
    workflow_dispatches: Arc<Mutex<Vec<serde_json::Value>>>,
    pull_requests: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    artifacts: Arc<Mutex<HashMap<i64, Vec<MockArtifact>>>>,
    check_runs: Arc<Mutex<Vec<serde_json::Value>>>,
    recorded_requests: Arc<Mutex<Vec<RecordedRequest>>>,
    next_check_run_id: Arc<Mutex<i64>>,
    workflow_dispatch_failure: Arc<Mutex<Option<MockFailureResponse>>>,
}

#[derive(Clone)]
struct MockArtifact {
    run_id: i64,
    id: i64,
    name: String,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct MockFailureResponse {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: String,
}

#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub user_agent: Option<String>,
}

pub struct Harness {
    state: mobench_webhook::AppState,
    workflow_dispatches: Arc<Mutex<Vec<serde_json::Value>>>,
    pull_requests: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    artifacts: Arc<Mutex<HashMap<i64, Vec<MockArtifact>>>>,
    check_runs: Arc<Mutex<Vec<serde_json::Value>>>,
    recorded_requests: Arc<Mutex<Vec<RecordedRequest>>>,
    workflow_dispatch_failure: Arc<Mutex<Option<MockFailureResponse>>>,
}

impl Harness {
    pub async fn new(pool: PgPool) -> Result<Self> {
        let workflow_dispatches = Arc::new(Mutex::new(Vec::new()));
        let pull_requests = Arc::new(Mutex::new(HashMap::new()));
        let artifacts = Arc::new(Mutex::new(HashMap::new()));
        let check_runs = Arc::new(Mutex::new(Vec::new()));
        let recorded_requests = Arc::new(Mutex::new(Vec::new()));
        let next_check_run_id = Arc::new(Mutex::new(9001));
        let workflow_dispatch_failure = Arc::new(Mutex::new(None));
        let server_state = MockGitHubState {
            workflow_dispatches: workflow_dispatches.clone(),
            pull_requests: pull_requests.clone(),
            artifacts: artifacts.clone(),
            check_runs: check_runs.clone(),
            recorded_requests: recorded_requests.clone(),
            next_check_run_id: next_check_run_id.clone(),
            workflow_dispatch_failure: workflow_dispatch_failure.clone(),
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
                "/repos/{owner}/{repo}/actions/runs/{run_id}/artifacts",
                get(list_artifacts),
            )
            .route(
                "/repos/{owner}/{repo}/actions/artifacts/{artifact_id}/zip",
                get(download_artifact),
            )
            .route(
                "/repos/{owner}/{repo}/pulls/{number}",
                get(fetch_pull_request),
            )
            .route("/repos/{owner}/{repo}/check-runs", post(create_check_run))
            .route(
                "/repos/{owner}/{repo}/check-runs/{check_run_id}",
                patch(update_check_run),
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
            artifacts,
            check_runs,
            recorded_requests,
            workflow_dispatch_failure,
        })
    }

    pub async fn enqueue_fixture(&self, fixture_name: &str) -> Result<()> {
        self.enqueue_fixture_as(fixture_name, fixture_name).await
    }

    pub async fn enqueue_fixture_as(&self, fixture_name: &str, delivery_id: &str) -> Result<()> {
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
            .insert_pending(delivery_id, event, action.as_deref(), payload)
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

    pub async fn list_deliveries(
        &self,
    ) -> Result<Vec<mobench_webhook::db::models::DeliveryRecord>> {
        self.state.repos().deliveries.list_all().await
    }

    pub async fn dispatched_workflows(&self) -> Vec<serde_json::Value> {
        self.workflow_dispatches.lock().await.clone()
    }

    pub async fn clear_dispatched_workflows(&self) {
        self.workflow_dispatches.lock().await.clear();
    }

    pub async fn stub_history_artifact(&self, fixture_dir: &str) -> Result<()> {
        self.stub_history_artifact_for_run(424242, fixture_dir)
            .await
    }

    pub async fn stub_history_artifact_for_run(
        &self,
        run_id: i64,
        fixture_dir: &str,
    ) -> Result<()> {
        let bytes = zip_fixture_directory(fixture_path(fixture_dir))?;
        let mut artifacts = self.artifacts.lock().await;
        artifacts.insert(
            run_id,
            vec![MockArtifact {
                run_id,
                id: 7001,
                name: "mobench-history-v1".to_string(),
                bytes,
            }],
        );

        Ok(())
    }

    pub async fn stub_history_artifact_with_overrides_for_run(
        &self,
        run_id: i64,
        fixture_dir: &str,
        overrides: &[(&str, &str)],
    ) -> Result<()> {
        let bytes = zip_fixture_directory_with_overrides(
            fixture_path(fixture_dir),
            overrides
                .iter()
                .map(|(path, contents)| ((*path).to_string(), contents.as_bytes().to_vec()))
                .collect(),
        )?;
        let mut artifacts = self.artifacts.lock().await;
        artifacts.insert(
            run_id,
            vec![MockArtifact {
                run_id,
                id: 7001,
                name: "mobench-history-v1".to_string(),
                bytes,
            }],
        );

        Ok(())
    }

    pub async fn stub_workflow_dispatch_rate_limited(&self, retry_after_secs: i32) {
        self.workflow_dispatch_failure
            .lock()
            .await
            .replace(MockFailureResponse {
                status: StatusCode::TOO_MANY_REQUESTS,
                headers: vec![("retry-after".to_string(), retry_after_secs.to_string())],
                body: serde_json::json!({
                    "message": "API rate limit exceeded"
                })
                .to_string(),
            });
    }

    pub async fn list_workflow_runs(
        &self,
    ) -> Result<Vec<mobench_webhook::db::models::WorkflowRunRecord>> {
        self.state.repos().runs.list_workflow_runs().await
    }

    pub async fn list_platform_runs(
        &self,
    ) -> Result<Vec<mobench_webhook::db::models::PlatformRunRecord>> {
        self.state.repos().runs.list_platform_runs().await
    }

    pub async fn list_results(
        &self,
    ) -> Result<Vec<mobench_webhook::db::models::BenchmarkResultRecord>> {
        self.state.repos().results.list_all().await
    }

    pub async fn recorded_check_runs(&self) -> Vec<serde_json::Value> {
        self.check_runs.lock().await.clone()
    }

    pub async fn recorded_requests(&self) -> Vec<RecordedRequest> {
        self.recorded_requests.lock().await.clone()
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
    Path((owner, repo, workflow_id)): Path<(String, String, String)>,
    State(state): State<MockGitHubState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    record_request(
        &state,
        "POST",
        format!("/repos/{owner}/{repo}/actions/workflows/{workflow_id}/dispatches"),
        &headers,
    )
    .await;
    if let Some(failure) = state.workflow_dispatch_failure.lock().await.take() {
        let mut response = (failure.status, failure.body).into_response();
        for (name, value) in failure.headers {
            response.headers_mut().insert(
                HeaderName::from_bytes(name.as_bytes()).expect("valid mock header name"),
                HeaderValue::from_str(&value).expect("valid mock header value"),
            );
        }

        return response;
    }

    state.workflow_dispatches.lock().await.push(payload);

    Json(serde_json::json!({})).into_response()
}

async fn fetch_pull_request(
    Path((owner, repo, number)): Path<(String, String, i32)>,
    State(state): State<MockGitHubState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    record_request(
        &state,
        "GET",
        format!("/repos/{owner}/{repo}/pulls/{number}"),
        &headers,
    )
    .await;
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

async fn list_artifacts(
    Path((owner, repo, run_id)): Path<(String, String, i64)>,
    State(state): State<MockGitHubState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    record_request(
        &state,
        "GET",
        format!("/repos/{owner}/{repo}/actions/runs/{run_id}/artifacts"),
        &headers,
    )
    .await;
    let artifacts = state.artifacts.lock().await;
    let items = artifacts.get(&run_id).cloned().unwrap_or_default();
    Json(serde_json::json!({
        "total_count": items.len(),
        "artifacts": items.iter().map(|artifact| {
            serde_json::json!({
                "id": artifact.id,
                "name": artifact.name
            })
        }).collect::<Vec<_>>()
    }))
}

async fn download_artifact(
    Path((owner, repo, artifact_id)): Path<(String, String, i64)>,
    State(state): State<MockGitHubState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    record_request(
        &state,
        "GET",
        format!("/repos/{owner}/{repo}/actions/artifacts/{artifact_id}/zip"),
        &headers,
    )
    .await;
    let artifact = state
        .artifacts
        .lock()
        .await
        .values()
        .flat_map(|entries| entries.iter())
        .find(|artifact| artifact.id == artifact_id)
        .cloned();

    match artifact {
        Some(artifact) => (
            StatusCode::OK,
            [("content-type", "application/zip")],
            artifact.bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn create_check_run(
    Path((owner, repo)): Path<(String, String)>,
    State(state): State<MockGitHubState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    record_request(
        &state,
        "POST",
        format!("/repos/{owner}/{repo}/check-runs"),
        &headers,
    )
    .await;
    let mut next_id = state.next_check_run_id.lock().await;
    let check_run_id = *next_id;
    *next_id += 1;
    let mut payload = payload;
    if let Some(object) = payload.as_object_mut() {
        object.insert("id".to_string(), serde_json::json!(check_run_id));
    }
    state.check_runs.lock().await.push(payload);

    Json(serde_json::json!({ "id": check_run_id }))
}

async fn update_check_run(
    Path((owner, repo, check_run_id)): Path<(String, String, i64)>,
    State(state): State<MockGitHubState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    record_request(
        &state,
        "PATCH",
        format!("/repos/{owner}/{repo}/check-runs/{check_run_id}"),
        &headers,
    )
    .await;
    let mut payload = payload;
    if let Some(object) = payload.as_object_mut() {
        object.insert("id".to_string(), serde_json::json!(check_run_id));
    }
    state.check_runs.lock().await.push(payload);

    Json(serde_json::json!({ "id": check_run_id }))
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

fn zip_fixture_directory(path: PathBuf) -> Result<Vec<u8>> {
    zip_fixture_directory_with_overrides(path, HashMap::new())
}

fn zip_fixture_directory_with_overrides(
    path: PathBuf,
    overrides: HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let options = FileOptions::default().compression_method(CompressionMethod::Deflated);
        add_directory_to_zip(&mut zip, &path, &path, options, &overrides)?;
        zip.finish().context("finishing fixture zip")?;
    }

    Ok(cursor.into_inner())
}

fn add_directory_to_zip(
    zip: &mut ZipWriter<&mut std::io::Cursor<Vec<u8>>>,
    root: &std::path::Path,
    dir: &std::path::Path,
    options: FileOptions,
    overrides: &HashMap<String, Vec<u8>>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            add_directory_to_zip(zip, root, &path, options, overrides)?;
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("computing relative path for {}", path.display()))?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        zip.start_file(relative.clone(), options)
            .context("starting zip file entry")?;
        let bytes = if let Some(bytes) = overrides.get(&relative) {
            bytes.clone()
        } else {
            std::fs::read(&path)
                .with_context(|| format!("reading fixture file {}", path.display()))?
        };
        use std::io::Write as _;
        zip.write_all(&bytes)
            .with_context(|| format!("writing fixture file {}", path.display()))?;
    }

    Ok(())
}

async fn record_request(state: &MockGitHubState, method: &str, path: String, headers: &HeaderMap) {
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    state.recorded_requests.lock().await.push(RecordedRequest {
        method: method.to_string(),
        path,
        user_agent,
    });
}
