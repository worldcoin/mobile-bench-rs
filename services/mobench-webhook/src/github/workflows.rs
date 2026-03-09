use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{Value, json};

use crate::github::{auth::GitHubAppAuth, into_api_result};

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
            http: Client::new(),
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
}
