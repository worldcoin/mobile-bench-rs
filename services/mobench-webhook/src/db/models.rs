use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct DeliveryRecord {
    pub id: Uuid,
    pub delivery_id: String,
    pub event: String,
    pub action: Option<String>,
    pub payload: Value,
    pub status: String,
    pub attempts: i32,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct BenchmarkDispatch {
    pub id: Uuid,
    pub dispatch_id: Uuid,
    pub repo_owner: String,
    pub repo_name: String,
    pub head_sha: String,
    pub head_ref: String,
    pub pr_number: Option<i32>,
    pub trigger_source: String,
    pub requested_by: Option<String>,
    pub workflow_inputs: Value,
    pub status: String,
    pub workflow_run_id: Option<i64>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct WorkflowRunRecord {
    pub id: Uuid,
    pub workflow_run_id: i64,
    pub workflow_run_attempt: i32,
    pub repo_owner: String,
    pub repo_name: String,
    pub workflow_name: String,
    pub head_sha: String,
    pub head_ref: String,
    pub base_ref: Option<String>,
    pub pr_number: Option<i32>,
    pub trigger_source: String,
    pub requested_by: Option<String>,
    pub request_command: Option<String>,
    pub mobench_version: Option<String>,
    pub mobench_ref: Option<String>,
    pub conclusion: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct PlatformRunRecord {
    pub id: Uuid,
    pub workflow_run_uuid: Uuid,
    pub platform: String,
    pub check_run_id: Option<i64>,
    pub check_run_name: String,
    pub workflow_inputs: Value,
    pub device_profile: Option<String>,
    pub device_name: String,
    pub os_version: String,
    pub iterations: i32,
    pub warmup: i32,
    pub status: String,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct BenchmarkResultRecord {
    pub id: Uuid,
    pub platform_run_uuid: Uuid,
    pub function_name: String,
    pub function_label: String,
    pub avg_ms: f64,
    pub median_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub best_ms: f64,
    pub worst_ms: f64,
    pub std_dev_ms: Option<f64>,
    pub cpu_avg_percent: Option<f64>,
    pub cpu_peak_percent: Option<f64>,
    pub ram_avg_mb: Option<f64>,
    pub ram_peak_mb: Option<f64>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct PlatformRunWithWorkflowRecord {
    pub id: Uuid,
    pub workflow_run_uuid: Uuid,
    pub platform: String,
    pub check_run_id: Option<i64>,
    pub check_run_name: String,
    pub workflow_inputs: Value,
    pub device_profile: Option<String>,
    pub device_name: String,
    pub os_version: String,
    pub iterations: i32,
    pub warmup: i32,
    pub status: String,
    pub workflow_run_id: i64,
    pub repo_owner: String,
    pub repo_name: String,
    pub head_sha: String,
    pub head_ref: String,
    pub base_ref: Option<String>,
    pub pr_number: Option<i32>,
    pub trigger_source: String,
    pub requested_by: Option<String>,
    pub request_command: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct TrendPointRecord {
    pub platform_run_id: Uuid,
    pub workflow_run_id: i64,
    pub head_sha: String,
    pub head_ref: String,
    pub function_name: String,
    pub function_label: String,
    pub avg_ms: f64,
    pub median_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub device_name: String,
    pub os_version: String,
}

#[derive(Debug, Clone)]
pub struct UpsertWorkflowRun<'a> {
    pub workflow_run_id: i64,
    pub workflow_run_attempt: i32,
    pub repo_owner: &'a str,
    pub repo_name: &'a str,
    pub workflow_name: &'a str,
    pub head_sha: &'a str,
    pub head_ref: &'a str,
    pub base_ref: Option<&'a str>,
    pub pr_number: Option<i32>,
    pub trigger_source: &'a str,
    pub requested_by: Option<&'a str>,
    pub request_command: Option<&'a str>,
    pub mobench_version: Option<&'a str>,
    pub mobench_ref: Option<&'a str>,
    pub conclusion: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct UpsertPlatformRun<'a> {
    pub workflow_run_uuid: Uuid,
    pub platform: &'a str,
    pub check_run_id: Option<i64>,
    pub check_run_name: &'a str,
    pub workflow_inputs: Value,
    pub device_profile: Option<&'a str>,
    pub device_name: &'a str,
    pub os_version: &'a str,
    pub iterations: i32,
    pub warmup: i32,
    pub status: &'a str,
}

#[derive(Debug, Clone)]
pub struct UpsertBenchmarkResult<'a> {
    pub platform_run_uuid: Uuid,
    pub function_name: &'a str,
    pub function_label: &'a str,
    pub avg_ms: f64,
    pub median_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub best_ms: f64,
    pub worst_ms: f64,
    pub std_dev_ms: Option<f64>,
    pub cpu_avg_percent: Option<f64>,
    pub cpu_peak_percent: Option<f64>,
    pub ram_avg_mb: Option<f64>,
    pub ram_peak_mb: Option<f64>,
}
