use std::convert::TryFrom;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    AppState,
    db::models::{
        DeliveryRecord, PlatformRunWithWorkflowRecord, UpsertBenchmarkResult, UpsertPlatformRun,
        UpsertWorkflowRun,
    },
    ingest::{self, HistoryBundle},
    webhook::commands::ManualRunArgs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Processed,
    Ignored,
}

pub async fn handle_delivery(
    state: &AppState,
    delivery: &DeliveryRecord,
) -> Result<DeliveryOutcome> {
    match (delivery.event.as_str(), delivery.action.as_deref()) {
        ("pull_request", Some("labeled")) => {
            handle_pull_request_labeled(state, &delivery.payload).await
        }
        ("issue_comment", Some("created")) => {
            handle_issue_comment_created(state, &delivery.payload).await
        }
        ("check_run", Some("rerequested")) => {
            handle_check_run_rerequested(state, &delivery.payload).await
        }
        ("workflow_run", Some("completed")) => {
            handle_workflow_run_completed(state, &delivery.payload).await
        }
        _ => Ok(DeliveryOutcome::Ignored),
    }
}

async fn handle_pull_request_labeled(state: &AppState, payload: &Value) -> Result<DeliveryOutcome> {
    let event = match serde_json::from_value::<PullRequestLabeledEvent>(payload.clone()) {
        Ok(event) => event,
        Err(_) => return Ok(DeliveryOutcome::Ignored),
    };
    match event.label.as_ref().map(|label| label.name.as_str()) {
        Some("bench") => {}
        _ => return Ok(DeliveryOutcome::Ignored),
    }

    if event.pull_request.state != "open" {
        return Ok(DeliveryOutcome::Ignored);
    }

    if event.pull_request.head.repo.full_name != event.pull_request.base.repo.full_name {
        return Ok(DeliveryOutcome::Ignored);
    }

    if !state.config.matches_repo(
        event.pull_request.base.repo.owner.login.as_str(),
        event.pull_request.base.repo.name.as_str(),
    ) {
        return Ok(DeliveryOutcome::Ignored);
    }

    let workflow_inputs = ManualRunArgs::default().workflow_inputs(
        event.number,
        &event.sender.login,
        &event.pull_request.base.reference,
    );
    let repo_owner = event.pull_request.base.repo.owner.login.as_str();
    let repo_name = event.pull_request.base.repo.name.as_str();
    let head_sha = event.pull_request.head.sha.as_str();
    let head_ref = event.pull_request.head.reference.as_str();

    if state
        .repos
        .dispatches
        .find_active_duplicate(repo_owner, repo_name, head_sha, &workflow_inputs)
        .await?
        .is_some()
    {
        info!(head_sha, pr_number = event.number, "dispatch.skipped");
        return Ok(DeliveryOutcome::Ignored);
    }

    let dispatch_id = Uuid::new_v4();
    let dispatch = state
        .repos
        .dispatches
        .insert_queued(
            dispatch_id,
            repo_owner,
            repo_name,
            head_sha,
            head_ref,
            Some(event.number),
            "label",
            Some(&event.sender.login),
            workflow_inputs.clone(),
        )
        .await?;
    let github = state
        .github
        .as_ref()
        .context("missing GitHub clients for dispatch handling")?;

    github
        .workflows
        .dispatch_mobile_bench(
            repo_owner,
            repo_name,
            &state.config.github_workflow_id,
            head_ref,
            &workflow_inputs_for_dispatch(dispatch.dispatch_id, "label", None, &workflow_inputs),
        )
        .await?;
    info!(
        head_sha,
        pr_number = event.number,
        trigger_source = "label",
        "dispatch.triggered"
    );

    Ok(DeliveryOutcome::Processed)
}

async fn handle_issue_comment_created(
    state: &AppState,
    payload: &Value,
) -> Result<DeliveryOutcome> {
    let event = match serde_json::from_value::<IssueCommentCreatedEvent>(payload.clone()) {
        Ok(event) => event,
        Err(_) => return Ok(DeliveryOutcome::Ignored),
    };
    if event.issue.pull_request.is_none() {
        return Ok(DeliveryOutcome::Ignored);
    }

    if !matches!(
        event.comment.author_association.as_str(),
        "OWNER" | "MEMBER" | "COLLABORATOR"
    ) {
        return Ok(DeliveryOutcome::Ignored);
    }

    let args = match ManualRunArgs::parse_manual_command(&event.comment.body) {
        Some(args) => args,
        None => return Ok(DeliveryOutcome::Ignored),
    };

    if !state.config.matches_repo(
        event.repository.owner.login.as_str(),
        event.repository.name.as_str(),
    ) {
        return Ok(DeliveryOutcome::Ignored);
    }

    let github = state
        .github
        .as_ref()
        .context("missing GitHub clients for dispatch handling")?;
    let repo_owner = event.repository.owner.login.as_str();
    let repo_name = event.repository.name.as_str();
    let pull_request = github
        .pull_requests
        .get_pull_request(repo_owner, repo_name, event.issue.number)
        .await?;
    if pull_request.state != "open" {
        return Ok(DeliveryOutcome::Ignored);
    }

    if pull_request.head.repo.full_name != pull_request.base.repo.full_name {
        return Ok(DeliveryOutcome::Ignored);
    }

    let workflow_inputs = args.workflow_inputs(
        event.issue.number,
        &event.comment.user.login,
        &pull_request.base.reference,
    );
    let head_sha = pull_request.head.sha.as_str();
    let head_ref = pull_request.head.reference.as_str();

    if state
        .repos
        .dispatches
        .find_active_duplicate(repo_owner, repo_name, head_sha, &workflow_inputs)
        .await?
        .is_some()
    {
        info!(head_sha, pr_number = event.issue.number, "dispatch.skipped");
        return Ok(DeliveryOutcome::Ignored);
    }

    let dispatch_id = Uuid::new_v4();
    let dispatch = state
        .repos
        .dispatches
        .insert_queued(
            dispatch_id,
            repo_owner,
            repo_name,
            head_sha,
            head_ref,
            Some(event.issue.number),
            "pr_comment",
            Some(&event.comment.user.login),
            workflow_inputs.clone(),
        )
        .await?;

    github
        .workflows
        .dispatch_mobile_bench(
            repo_owner,
            repo_name,
            &state.config.github_workflow_id,
            head_ref,
            &workflow_inputs_for_dispatch(
                dispatch.dispatch_id,
                "pr_comment",
                Some(event.comment.body.trim()),
                &workflow_inputs,
            ),
        )
        .await?;
    info!(
        head_sha,
        pr_number = event.issue.number,
        trigger_source = "pr_comment",
        "dispatch.triggered"
    );

    Ok(DeliveryOutcome::Processed)
}

async fn handle_check_run_rerequested(
    state: &AppState,
    payload: &Value,
) -> Result<DeliveryOutcome> {
    let event = match serde_json::from_value::<CheckRunRerequestedEvent>(payload.clone()) {
        Ok(event) => event,
        Err(_) => return Ok(DeliveryOutcome::Ignored),
    };
    if event.check_run.app.id != state.config.github_app_id {
        return Ok(DeliveryOutcome::Ignored);
    }

    let Some(platform_run) = state
        .repos
        .runs
        .get_platform_run_by_check_run_id(event.check_run.id)
        .await?
    else {
        warn!(check_run_id = event.check_run.id, "check_run.unknown");
        return Ok(DeliveryOutcome::Ignored);
    };
    let github = state
        .github
        .as_ref()
        .context("missing GitHub clients for dispatch handling")?;
    let mut workflow_inputs = platform_run.workflow_inputs.clone();
    if let Some(object) = workflow_inputs.as_object_mut() {
        object.insert(
            "platform".to_string(),
            Value::String(platform_run.platform.clone()),
        );
        if let Some(sender) = event.sender.as_ref() {
            object.insert(
                "requested_by".to_string(),
                Value::String(sender.login.clone()),
            );
        }
    }

    let requested_by = event.sender.as_ref().map(|sender| sender.login.as_str());
    let dispatch_id = Uuid::new_v4();
    let dispatch = state
        .repos
        .dispatches
        .insert_queued(
            dispatch_id,
            platform_run.repo_owner.as_str(),
            platform_run.repo_name.as_str(),
            platform_run.head_sha.as_str(),
            platform_run.head_ref.as_str(),
            platform_run.pr_number,
            "check_rerequest",
            requested_by,
            workflow_inputs.clone(),
        )
        .await?;

    github
        .workflows
        .dispatch_mobile_bench(
            platform_run.repo_owner.as_str(),
            platform_run.repo_name.as_str(),
            &state.config.github_workflow_id,
            platform_run.head_ref.as_str(),
            &workflow_inputs_for_dispatch(
                dispatch.dispatch_id,
                "check_rerequest",
                platform_run.request_command.as_deref(),
                &workflow_inputs,
            ),
        )
        .await?;
    info!(
        check_run_id = event.check_run.id,
        workflow_run_id = platform_run.workflow_run_id,
        trigger_source = "check_rerequest",
        "dispatch.triggered"
    );

    Ok(DeliveryOutcome::Processed)
}

async fn handle_workflow_run_completed(
    state: &AppState,
    payload: &Value,
) -> Result<DeliveryOutcome> {
    let event = match serde_json::from_value::<WorkflowRunCompletedEvent>(payload.clone()) {
        Ok(event) => event,
        Err(_) => return Ok(DeliveryOutcome::Ignored),
    };
    if !matches_workflow_run(state, &event.workflow_run) {
        return Ok(DeliveryOutcome::Ignored);
    }

    if !state.config.matches_repo(
        event.repository.owner.login.as_str(),
        event.repository.name.as_str(),
    ) {
        return Ok(DeliveryOutcome::Ignored);
    }

    let github = state
        .github
        .as_ref()
        .context("missing GitHub clients for ingest handling")?;
    let repo_owner = event.repository.owner.login.as_str();
    let repo_name = event.repository.name.as_str();
    let correlated_dispatch_id = state
        .repos
        .dispatches
        .find_latest_inflight_for_head(
            repo_owner,
            repo_name,
            event.workflow_run.head_sha.as_str(),
            event.workflow_run.head_branch.as_str(),
        )
        .await?
        .map(|dispatch| dispatch.dispatch_id);
    info!(workflow_run_id = event.workflow_run.id, "ingest.started");
    let bundle_bytes = match github
        .artifacts
        .download_history_bundle(repo_owner, repo_name, event.workflow_run.id)
        .await
    {
        Ok(bytes) => bytes,
        Err(err) => {
            attach_correlated_dispatch(
                state,
                correlated_dispatch_id,
                event.workflow_run.id,
                "failed",
            )
            .await?;
            return Err(err);
        }
    };
    info!(
        workflow_run_id = event.workflow_run.id,
        artifact_name = "mobench-history-v1",
        "ingest.bundle_fetched"
    );
    let bundle = match HistoryBundle::from_zip(&bundle_bytes) {
        Ok(bundle) => bundle,
        Err(err) => {
            attach_correlated_dispatch(
                state,
                correlated_dispatch_id,
                event.workflow_run.id,
                "failed",
            )
            .await?;
            return Err(err);
        }
    };
    let manifest: ingest::manifest::HistoryManifest = match bundle.read_json("manifest.json") {
        Ok(manifest) => manifest,
        Err(err) => {
            attach_correlated_dispatch(
                state,
                correlated_dispatch_id,
                event.workflow_run.id,
                "failed",
            )
            .await?;
            return Err(err);
        }
    };
    let base_ref = ingest::compare::preferred_base_ref(Some(manifest.git.base_ref.as_str()));
    let workflow_run = state
        .repos
        .runs
        .upsert_workflow_run(UpsertWorkflowRun {
            workflow_run_id: i64::try_from(manifest.workflow.run_id)
                .context("manifest workflow.run_id exceeds i64")?,
            workflow_run_attempt: i32::try_from(manifest.workflow.run_attempt)
                .context("manifest workflow.run_attempt exceeds i32")?,
            repo_owner,
            repo_name,
            workflow_name: manifest.workflow.name.as_str(),
            head_sha: manifest.git.head_sha.as_str(),
            head_ref: manifest.git.head_ref.as_str(),
            base_ref: Some(base_ref),
            pr_number: manifest
                .request
                .pr_number
                .map(i32::try_from)
                .transpose()
                .context("manifest request.pr_number exceeds i32")?,
            trigger_source: manifest.request.trigger_source.as_str(),
            requested_by: manifest.request.requested_by.as_deref(),
            request_command: manifest.request.request_command.as_deref(),
            mobench_version: Some(manifest.mobench.version.as_str()),
            mobench_ref: Some(manifest.mobench.mobench_ref.as_str()),
            conclusion: event.workflow_run.conclusion.as_deref(),
        })
        .await?;

    let dispatch_id = manifest.request.dispatch_id.or(correlated_dispatch_id);
    attach_correlated_dispatch(state, dispatch_id, workflow_run.workflow_run_id, "running").await?;

    let mut successful_platforms = 0usize;
    let mut platform_failures = Vec::new();

    for platform_run in &manifest.platform_runs {
        match ingest_platform_run(
            state,
            github,
            &bundle,
            &workflow_run,
            platform_run,
            base_ref,
            repo_owner,
            repo_name,
        )
        .await
        {
            Ok(()) => successful_platforms += 1,
            Err(err) => {
                warn!(
                    workflow_run_id = workflow_run.workflow_run_id,
                    platform = platform_run.platform.as_str(),
                    error = err.to_string(),
                    "ingest.failed"
                );
                persist_failed_platform_run(state, workflow_run.id, platform_run).await?;
                platform_failures.push(format!("{}: {}", platform_run.platform, err));
            }
        }
    }

    if successful_platforms == 0 {
        attach_correlated_dispatch(state, dispatch_id, workflow_run.workflow_run_id, "failed")
            .await?;
        return Err(anyhow!(
            "failed to ingest any platform results for workflow run {}: {}",
            workflow_run.workflow_run_id,
            platform_failures.join("; ")
        ));
    }

    attach_correlated_dispatch(
        state,
        dispatch_id,
        workflow_run.workflow_run_id,
        "completed",
    )
    .await?;

    info!(
        workflow_run_id = workflow_run.workflow_run_id,
        platform_runs = successful_platforms,
        "ingest.completed"
    );

    Ok(DeliveryOutcome::Processed)
}

async fn ingest_platform_run(
    state: &AppState,
    github: &crate::github::GitHubClients,
    bundle: &HistoryBundle,
    workflow_run: &crate::db::models::WorkflowRunRecord,
    platform_run: &ingest::manifest::PlatformRun,
    base_ref: &str,
    repo_owner: &str,
    repo_name: &str,
) -> Result<()> {
    let summary_bytes = bundle
        .read_json::<serde_json::Value>(&format!("{}/summary.json", platform_run.platform))?;
    let summary_markdown = bundle
        .read_text(&format!("{}/summary.md", platform_run.platform))
        .unwrap_or_default();
    let report = ingest::summary::parse_summary_json(
        serde_json::to_string(&summary_bytes)
            .context("serializing platform summary value")?
            .as_bytes(),
    )?;
    let summary_platform = report
        .platforms
        .iter()
        .find(|candidate| candidate.platform == platform_run.platform)
        .or_else(|| report.platforms.first())
        .with_context(|| format!("missing platform report for {}", platform_run.platform))?;

    let persisted_platform_run = state
        .repos
        .runs
        .upsert_platform_run(UpsertPlatformRun {
            workflow_run_uuid: workflow_run.id,
            platform: platform_run.platform.as_str(),
            check_run_id: None,
            check_run_name: platform_run.check_run_name.as_str(),
            workflow_inputs: serde_json::to_value(&platform_run.workflow_inputs)
                .context("serializing workflow inputs")?,
            device_profile: platform_run
                .workflow_inputs
                .get("device_profile")
                .map(String::as_str),
            device_name: platform_run.resolved_device.device_name.as_str(),
            os_version: platform_run.resolved_device.os_version.as_str(),
            iterations: i32::try_from(summary_platform.iterations)
                .context("summary iterations exceed i32")?,
            warmup: i32::try_from(summary_platform.warmup).context("summary warmup exceed i32")?,
            status: "completed",
        })
        .await?;

    state
        .repos
        .results
        .delete_for_platform_run(persisted_platform_run.id)
        .await?;

    for benchmark in &summary_platform.benchmarks {
        state
            .repos
            .results
            .upsert_result(UpsertBenchmarkResult {
                platform_run_uuid: persisted_platform_run.id,
                function_name: benchmark.name.as_str(),
                function_label: benchmark.label.as_str(),
                avg_ms: benchmark.timing.avg_ms,
                median_ms: Some(benchmark.timing.median_ms),
                p95_ms: Some(benchmark.timing.p95_ms),
                best_ms: benchmark.timing.best_ms,
                worst_ms: benchmark.timing.worst_ms,
                std_dev_ms: Some(benchmark.timing.std_dev_ms),
                cpu_avg_percent: benchmark
                    .resource_usage
                    .as_ref()
                    .and_then(|usage| usage.cpu_avg_percent),
                cpu_peak_percent: benchmark
                    .resource_usage
                    .as_ref()
                    .and_then(|usage| usage.cpu_peak_percent),
                ram_avg_mb: benchmark
                    .resource_usage
                    .as_ref()
                    .and_then(|usage| usage.ram_avg_mb),
                ram_peak_mb: benchmark
                    .resource_usage
                    .as_ref()
                    .and_then(|usage| usage.ram_peak_mb),
            })
            .await?;
    }

    let baseline = load_baseline_platform_run(state, persisted_platform_run.id, base_ref).await?;
    let baseline_results = if let Some(run) = baseline.as_ref() {
        state.repos.results.list_for_platform_run(run.id).await?
    } else {
        Vec::new()
    };
    let candidate_results = state
        .repos
        .results
        .list_for_platform_run(persisted_platform_run.id)
        .await?;
    let comparison_rows = ingest::compare::compare_result_sets(
        &baseline_results,
        &candidate_results,
        parse_workflow_input_f64(
            &platform_run.workflow_inputs,
            "regression_threshold_pct",
            5.0,
        ),
    );
    let check_run_payload = build_check_run_payload(
        workflow_run,
        platform_run.check_run_name.as_str(),
        &summary_markdown,
        &comparison_rows,
        &state.config.github_workflow_id,
    );
    let check_run_response = github
        .checks
        .create_or_update_check_run(
            repo_owner,
            repo_name,
            persisted_platform_run.check_run_id,
            &check_run_payload,
        )
        .await?;
    let check_run_id = check_run_response
        .get("id")
        .and_then(Value::as_i64)
        .context("missing check run id in GitHub response")?;
    state
        .repos
        .runs
        .set_check_run_id(persisted_platform_run.id, check_run_id)
        .await?;
    info!(
        workflow_run_id = workflow_run.workflow_run_id,
        platform = platform_run.platform.as_str(),
        benchmarks = candidate_results.len(),
        "ingest.platform_done"
    );
    info!(
        platform_run_id = persisted_platform_run.id.to_string(),
        check_run_id, "check_run.upserted"
    );

    Ok(())
}

async fn persist_failed_platform_run(
    state: &AppState,
    workflow_run_uuid: Uuid,
    platform_run: &ingest::manifest::PlatformRun,
) -> Result<()> {
    let failed_platform_run = state
        .repos
        .runs
        .upsert_platform_run(UpsertPlatformRun {
            workflow_run_uuid,
            platform: platform_run.platform.as_str(),
            check_run_id: None,
            check_run_name: platform_run.check_run_name.as_str(),
            workflow_inputs: serde_json::to_value(&platform_run.workflow_inputs)
                .context("serializing workflow inputs")?,
            device_profile: platform_run
                .workflow_inputs
                .get("device_profile")
                .map(String::as_str),
            device_name: platform_run.resolved_device.device_name.as_str(),
            os_version: platform_run.resolved_device.os_version.as_str(),
            iterations: parse_workflow_input_i32(&platform_run.workflow_inputs, "iterations"),
            warmup: parse_workflow_input_i32(&platform_run.workflow_inputs, "warmup"),
            status: "failed",
        })
        .await?;
    state
        .repos
        .results
        .delete_for_platform_run(failed_platform_run.id)
        .await?;

    Ok(())
}

fn parse_workflow_input_i32(
    workflow_inputs: &std::collections::BTreeMap<String, String>,
    key: &str,
) -> i32 {
    workflow_inputs
        .get(key)
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or_default()
}

fn parse_workflow_input_f64(
    workflow_inputs: &std::collections::BTreeMap<String, String>,
    key: &str,
    default: f64,
) -> f64 {
    workflow_inputs
        .get(key)
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

fn workflow_inputs_for_dispatch(
    dispatch_id: Uuid,
    trigger_source: &str,
    request_command: Option<&str>,
    workflow_inputs: &Value,
) -> Value {
    let mut workflow_inputs = workflow_inputs.clone();
    if let Some(object) = workflow_inputs.as_object_mut() {
        object.insert(
            "dispatch_id".to_string(),
            Value::String(dispatch_id.to_string()),
        );
        object.insert(
            "trigger_source".to_string(),
            Value::String(trigger_source.to_string()),
        );
        if let Some(request_command) = request_command {
            object.insert(
                "request_command".to_string(),
                Value::String(request_command.to_string()),
            );
        }
    }

    workflow_inputs
}

#[derive(Debug, Deserialize)]
struct PullRequestLabeledEvent {
    number: i32,
    label: Option<EventLabel>,
    pull_request: PullRequestSummary,
    sender: GitHubUser,
}

#[derive(Debug, Deserialize)]
struct IssueCommentCreatedEvent {
    repository: GitHubRepo,
    issue: IssueSummary,
    comment: IssueComment,
}

#[derive(Debug, Deserialize)]
struct EventLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct IssueSummary {
    number: i32,
    pull_request: Option<IssuePullRequestContext>,
}

#[derive(Debug, Deserialize)]
struct IssueComment {
    body: String,
    author_association: String,
    user: GitHubUser,
}

#[derive(Debug, Deserialize)]
struct IssuePullRequestContext {}

#[derive(Debug, Deserialize)]
struct WorkflowRunCompletedEvent {
    repository: GitHubRepo,
    workflow_run: CompletedWorkflowRun,
}

#[derive(Debug, Deserialize)]
struct CompletedWorkflowRun {
    id: i64,
    name: String,
    path: Option<String>,
    head_sha: String,
    head_branch: String,
    conclusion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CheckRunRerequestedEvent {
    check_run: RerequestedCheckRun,
    sender: Option<GitHubUser>,
}

#[derive(Debug, Deserialize)]
struct RerequestedCheckRun {
    id: i64,
    app: CheckRunApp,
}

#[derive(Debug, Deserialize)]
struct CheckRunApp {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct PullRequestSummary {
    state: String,
    head: GitHubRef,
    base: GitHubBaseRef,
}

#[derive(Debug, Deserialize)]
struct GitHubRef {
    #[serde(rename = "ref")]
    reference: String,
    sha: String,
    repo: GitHubRepo,
}

#[derive(Debug, Deserialize)]
struct GitHubBaseRef {
    #[serde(rename = "ref")]
    reference: String,
    repo: GitHubRepo,
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    name: String,
    full_name: String,
    owner: GitHubUser,
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
}

fn matches_workflow_run(state: &AppState, workflow_run: &CompletedWorkflowRun) -> bool {
    if let Some(path) = workflow_run.path.as_deref()
        && path.rsplit('/').next() == Some(state.config.github_workflow_id.as_str())
    {
        return true;
    }

    workflow_run.name == "Mobile Benchmarks" || workflow_run.name == "Mobile Bench (manual)"
}

async fn load_baseline_platform_run(
    state: &AppState,
    platform_run_uuid: Uuid,
    base_ref: &str,
) -> Result<Option<PlatformRunWithWorkflowRecord>> {
    if let Some(run) = state
        .repos
        .runs
        .find_latest_successful_baseline(platform_run_uuid, base_ref)
        .await?
    {
        return Ok(Some(run));
    }

    if base_ref != "main" {
        return state
            .repos
            .runs
            .find_latest_successful_baseline(platform_run_uuid, "main")
            .await;
    }

    Ok(None)
}

fn build_check_run_payload(
    workflow_run: &crate::db::models::WorkflowRunRecord,
    check_run_name: &str,
    summary_markdown: &str,
    comparison_rows: &[ingest::compare::ComparisonRow],
    workflow_id: &str,
) -> Value {
    let regressions = comparison_rows
        .iter()
        .filter(|row| row.label == "regressed")
        .collect::<Vec<_>>();
    let bench_count = comparison_rows.len();
    let title = if regressions.is_empty() {
        format!("{bench_count} benchmarks passed")
    } else {
        format!("{bench_count} benchmarks - {} regressed", regressions.len())
    };
    let conclusion = if regressions.is_empty() {
        "success"
    } else {
        "failure"
    };
    let mut summary = summary_markdown.trim().to_string();
    if !comparison_rows.is_empty() {
        if !summary.is_empty() {
            summary.push_str("\n\n");
        }
        summary.push_str("## Baseline Comparison\n\n");
        summary.push_str("| Benchmark | Baseline ms | Candidate ms | Delta % | Status |\n");
        summary.push_str("| --- | ---: | ---: | ---: | --- |\n");
        for row in comparison_rows {
            let baseline = row
                .baseline_avg_ms
                .map(|value| format!("{value:.1}"))
                .unwrap_or_else(|| "-".to_string());
            let delta = row
                .delta_pct
                .map(|value| format!("{value:+.1}"))
                .unwrap_or_else(|| "-".to_string());
            summary.push_str(&format!(
                "| {} | {} | {:.1} | {} | {} |\n",
                row.function_label, baseline, row.candidate_avg_ms, delta, row.label
            ));
        }
    }

    let annotations = regressions
        .iter()
        .enumerate()
        .map(|(index, row)| {
            json!({
                "path": format!(".github/workflows/{workflow_id}"),
                "start_line": index + 1,
                "end_line": index + 1,
                "annotation_level": "warning",
                "message": format!(
                    "{} regressed {}% ({} -> {:.1}ms)",
                    row.function_label,
                    row.delta_pct.unwrap_or_default().round(),
                    row.baseline_avg_ms
                        .map(|value| format!("{value:.1}ms"))
                        .unwrap_or_else(|| "no baseline".to_string()),
                    row.candidate_avg_ms
                ),
                "title": format!("Regression: {}", row.function_label)
            })
        })
        .collect::<Vec<_>>();

    json!({
        "name": check_run_name,
        "head_sha": workflow_run.head_sha,
        "status": "completed",
        "conclusion": conclusion,
        "output": {
            "title": title,
            "summary": summary,
            "annotations": annotations
        }
    })
}

async fn attach_correlated_dispatch(
    state: &AppState,
    dispatch_id: Option<Uuid>,
    workflow_run_id: i64,
    status: &str,
) -> Result<()> {
    if let Some(dispatch_id) = dispatch_id {
        state
            .repos
            .dispatches
            .attach_workflow_run(dispatch_id, workflow_run_id, status)
            .await?;
    }

    Ok(())
}
