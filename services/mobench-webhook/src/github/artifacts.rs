use anyhow::{Context, Result};
use reqwest::Client;

use crate::github::auth::GitHubAppAuth;

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
            http: Client::new(),
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
            .context("downloading GitHub artifact")?
            .error_for_status()
            .context("GitHub artifact download failed")?;

        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .context("reading GitHub artifact response bytes")
    }
}
