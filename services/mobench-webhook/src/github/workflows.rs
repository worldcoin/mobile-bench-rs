use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::github::{api_http_client, auth::GitHubAppAuth, into_api_result};

#[derive(Clone)]
pub struct GitHubWorkflowsClient {
    auth: GitHubAppAuth,
    http: Client,
    api_base_url: String,
}

impl GitHubWorkflowsClient {
    pub fn new(auth: GitHubAppAuth, api_base_url: impl Into<String>) -> Self {
        Self {
            auth,
            http: api_http_client(),
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub async fn dispatch_workflow(
        &self,
        owner: &str,
        repo: &str,
        workflow_id: &str,
        git_ref: &str,
        inputs: &Value,
    ) -> Result<()> {
        let token = self.auth.installation_token().await?;
        let response = self
            .http
            .post(format!(
                "{}/repos/{owner}/{repo}/actions/workflows/{workflow_id}/dispatches",
                self.api_base_url
            ))
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(token)
            .json(&json!({
                "ref": git_ref,
                "inputs": inputs,
            }))
            .send()
            .await
            .context("dispatching GitHub workflow")?;
        into_api_result(response, "GitHub workflow dispatch failed").await?;

        Ok(())
    }

    pub async fn dispatch_mobile_bench(
        &self,
        owner: &str,
        repo: &str,
        workflow_id: &str,
        git_ref: &str,
        inputs: &Value,
    ) -> Result<()> {
        self.dispatch_workflow(owner, repo, workflow_id, git_ref, inputs)
            .await
    }

    pub async fn has_successful_workflow_run_for_head(
        &self,
        owner: &str,
        repo: &str,
        workflow_id: &str,
        head_sha: &str,
    ) -> Result<bool> {
        let token = self.auth.installation_token().await?;
        let response = self
            .http
            .get(format!(
                "{}/repos/{owner}/{repo}/actions/workflows/{workflow_id}/runs",
                self.api_base_url
            ))
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(token)
            .query(&[
                ("head_sha", head_sha),
                ("status", "completed"),
                ("per_page", "100"),
            ])
            .send()
            .await
            .context("listing GitHub workflow runs")?;
        let response = into_api_result(response, "GitHub workflow run lookup failed").await?;
        let payload: WorkflowRunsResponse = response
            .json()
            .await
            .context("decoding GitHub workflow run response")?;

        Ok(payload.workflow_runs.into_iter().any(|run| {
            run.head_sha == head_sha && run.conclusion.as_deref() == Some("success")
        }))
    }
}

#[derive(Debug, Deserialize)]
struct WorkflowRunsResponse {
    workflow_runs: Vec<WorkflowRunSummary>,
}

#[derive(Debug, Deserialize)]
struct WorkflowRunSummary {
    head_sha: String,
    conclusion: Option<String>,
}
