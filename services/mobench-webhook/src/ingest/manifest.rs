use serde::Deserialize;
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct HistoryManifest {
    pub schema_version: String,
    pub repo: String,
    pub workflow: WorkflowMetadata,
    pub git: GitMetadata,
    pub request: HistoryRequest,
    pub mobench: MobenchMetadata,
    pub platform_runs: Vec<PlatformRun>,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowMetadata {
    pub name: String,
    pub run_id: u64,
    pub run_attempt: u32,
}

#[derive(Debug, Deserialize)]
pub struct GitMetadata {
    pub head_sha: String,
    pub head_ref: String,
    pub base_ref: String,
}

#[derive(Debug, Deserialize)]
pub struct HistoryRequest {
    pub dispatch_id: Option<Uuid>,
    pub trigger_source: String,
    pub requested_by: Option<String>,
    pub pr_number: Option<u64>,
    pub request_command: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MobenchMetadata {
    pub version: String,
    #[serde(rename = "ref")]
    pub mobench_ref: String,
}

#[derive(Debug, Deserialize)]
pub struct PlatformRun {
    pub platform: String,
    pub check_run_name: String,
    pub workflow_inputs: BTreeMap<String, String>,
    pub resolved_device: ResolvedDevice,
}

#[derive(Debug, Deserialize)]
pub struct ResolvedDevice {
    pub device_name: String,
    pub os_version: String,
}
