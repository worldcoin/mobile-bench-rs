use anyhow::Result;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState,
    db::models::{
        BenchmarkResultRecord, PlatformRunRecord, PlatformRunWithWorkflowRecord, TrendPointRecord,
        WorkflowRunRecord,
    },
    ingest::compare,
};

#[derive(Debug, Deserialize)]
pub struct WorkflowRunListQuery {
    branch: Option<String>,
    pr_number: Option<i32>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct TrendsQuery {
    function: String,
    platform: String,
    device_name: String,
    branch: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CompareQuery {
    platform_run_id: Uuid,
    baseline_branch: Option<String>,
    threshold_pct: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct WorkflowRunListResponse {
    workflow_runs: Vec<WorkflowRunSummary>,
}

#[derive(Debug, Serialize)]
pub struct WorkflowRunSummary {
    #[serde(flatten)]
    workflow_run: WorkflowRunRecord,
    platform_runs: Vec<PlatformRunRecord>,
}

#[derive(Debug, Serialize)]
pub struct WorkflowRunDetailResponse {
    workflow_run: WorkflowRunSummary,
}

#[derive(Debug, Serialize)]
pub struct PlatformRunDetailResponse {
    platform_run: PlatformRunDetail,
}

#[derive(Debug, Serialize)]
pub struct PlatformRunDetail {
    #[serde(flatten)]
    platform_run: PlatformRunWithWorkflowRecord,
    results: Vec<BenchmarkResultRecord>,
}

#[derive(Debug, Serialize)]
pub struct TrendsResponse {
    points: Vec<TrendPointRecord>,
}

#[derive(Debug, Serialize)]
pub struct CompareResponse {
    platform_run_id: Uuid,
    baseline_platform_run_id: Option<Uuid>,
    baseline_branch: String,
    threshold_pct: f64,
    rows: Vec<compare::ComparisonRow>,
}

pub async fn list_workflow_runs(
    State(state): State<AppState>,
    Query(query): Query<WorkflowRunListQuery>,
) -> Result<Json<WorkflowRunListResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(20).clamp(1, 200);
    let workflow_runs = state
        .repos
        .runs
        .list_workflow_runs_filtered(query.branch.as_deref(), query.pr_number, limit)
        .await
        .map_err(internal_error)?;
    let mut summaries = Vec::with_capacity(workflow_runs.len());

    for workflow_run in workflow_runs {
        let platform_runs = state
            .repos
            .runs
            .list_platform_runs_for_workflow(workflow_run.id)
            .await
            .map_err(internal_error)?;
        summaries.push(WorkflowRunSummary {
            workflow_run,
            platform_runs,
        });
    }

    Ok(Json(WorkflowRunListResponse {
        workflow_runs: summaries,
    }))
}

pub async fn get_workflow_run(
    State(state): State<AppState>,
    Path(workflow_run_id): Path<i64>,
) -> Result<Json<WorkflowRunDetailResponse>, StatusCode> {
    let Some(workflow_run) = state
        .repos
        .runs
        .get_workflow_run(workflow_run_id)
        .await
        .map_err(internal_error)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let platform_runs = state
        .repos
        .runs
        .list_platform_runs_for_workflow(workflow_run.id)
        .await
        .map_err(internal_error)?;

    Ok(Json(WorkflowRunDetailResponse {
        workflow_run: WorkflowRunSummary {
            workflow_run,
            platform_runs,
        },
    }))
}

pub async fn get_platform_run(
    State(state): State<AppState>,
    Path(platform_run_id): Path<Uuid>,
) -> Result<Json<PlatformRunDetailResponse>, StatusCode> {
    let Some(platform_run) = state
        .repos
        .runs
        .get_platform_run_with_workflow(platform_run_id)
        .await
        .map_err(internal_error)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let results = state
        .repos
        .results
        .list_for_platform_run(platform_run.id)
        .await
        .map_err(internal_error)?;

    Ok(Json(PlatformRunDetailResponse {
        platform_run: PlatformRunDetail {
            platform_run,
            results,
        },
    }))
}

pub async fn get_trends(
    State(state): State<AppState>,
    Query(query): Query<TrendsQuery>,
) -> Result<Json<TrendsResponse>, StatusCode> {
    let points = state
        .repos
        .runs
        .list_trend_points(
            &query.function,
            &query.platform,
            &query.device_name,
            query.branch.as_deref(),
            query.limit.unwrap_or(50).clamp(1, 200),
        )
        .await
        .map_err(internal_error)?;

    Ok(Json(TrendsResponse { points }))
}

pub async fn compare_platform_run(
    State(state): State<AppState>,
    Query(query): Query<CompareQuery>,
) -> Result<Json<CompareResponse>, StatusCode> {
    let Some(candidate) = state
        .repos
        .runs
        .get_platform_run_with_workflow(query.platform_run_id)
        .await
        .map_err(internal_error)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let baseline_branch = query
        .baseline_branch
        .unwrap_or_else(|| compare::preferred_base_ref(candidate.base_ref.as_deref()).to_string());
    let threshold_pct = query.threshold_pct.unwrap_or(5.0);
    let baseline = state
        .repos
        .runs
        .find_latest_successful_baseline(candidate.id, &baseline_branch)
        .await
        .map_err(internal_error)?;
    let candidate_results = state
        .repos
        .results
        .list_for_platform_run(candidate.id)
        .await
        .map_err(internal_error)?;
    let baseline_results = if let Some(baseline) = baseline.as_ref() {
        state
            .repos
            .results
            .list_for_platform_run(baseline.id)
            .await
            .map_err(internal_error)?
    } else {
        Vec::new()
    };

    Ok(Json(CompareResponse {
        platform_run_id: candidate.id,
        baseline_platform_run_id: baseline.as_ref().map(|run| run.id),
        baseline_branch,
        threshold_pct,
        rows: compare::compare_result_sets(&baseline_results, &candidate_results, threshold_pct),
    }))
}

fn internal_error(_err: anyhow::Error) -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}
