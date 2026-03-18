use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::Deserialize;

use crate::github::{api_http_client, auth::GitHubAppAuth, into_api_result};

#[derive(Clone)]
pub struct GitHubArtifactsClient {
    auth: GitHubAppAuth,
    http: Client,
    api_base_url: String,
}

impl GitHubArtifactsClient {
    pub fn new(auth: GitHubAppAuth, api_base_url: impl Into<String>) -> Self {
        Self {
            auth,
            http: api_http_client(),
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub async fn download_artifact(
        &self,
        owner: &str,
        repo: &str,
        artifact_id: i64,
    ) -> Result<Vec<u8>> {
        let token = self.auth.installation_token().await?;
        let response = self
            .http
            .get(format!(
                "{}/repos/{owner}/{repo}/actions/artifacts/{artifact_id}/zip",
                self.api_base_url
            ))
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(token)
            .send()
            .await
            .context("downloading GitHub artifact")?;
        let response = into_api_result(response, "GitHub artifact download failed").await?;

        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .context("reading GitHub artifact response bytes")
    }

    pub async fn download_history_bundle(
        &self,
        owner: &str,
        repo: &str,
        run_id: i64,
    ) -> Result<Vec<u8>> {
        let artifacts = self
            .list_workflow_run_artifacts(owner, repo, run_id)
            .await?;
        let artifact = artifacts
            .into_iter()
            .find(|artifact| artifact.name == "mobench-history-v1")
            .ok_or_else(|| {
                anyhow!("missing mobench-history-v1 artifact for workflow run {run_id}")
            })?;

        self.download_artifact(owner, repo, artifact.id).await
    }

    async fn list_workflow_run_artifacts(
        &self,
        owner: &str,
        repo: &str,
        run_id: i64,
    ) -> Result<Vec<ArtifactSummary>> {
        let token = self.auth.installation_token().await?;
        let response = self
            .http
            .get(format!(
                "{}/repos/{owner}/{repo}/actions/runs/{run_id}/artifacts",
                self.api_base_url
            ))
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(token)
            .send()
            .await
            .context("listing GitHub workflow artifacts")?;
        let response = into_api_result(response, "GitHub workflow artifact list failed").await?;
        let payload: WorkflowArtifactsResponse = response
            .json()
            .await
            .context("decoding GitHub workflow artifact list response")?;

        Ok(payload.artifacts)
    }
}

#[derive(Debug, Deserialize)]
struct WorkflowArtifactsResponse {
    artifacts: Vec<ArtifactSummary>,
}

#[derive(Debug, Deserialize)]
struct ArtifactSummary {
    id: i64,
    name: String,
}
