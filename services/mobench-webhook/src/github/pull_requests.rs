use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::github::{api_http_client, auth::GitHubAppAuth, into_api_result};

#[derive(Clone)]
pub struct GitHubPullRequestsClient {
    auth: GitHubAppAuth,
    http: Client,
    api_base_url: String,
}

impl GitHubPullRequestsClient {
    pub fn new(auth: GitHubAppAuth, api_base_url: impl Into<String>) -> Self {
        Self {
            auth,
            http: api_http_client(),
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub async fn get_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
    ) -> Result<PullRequestDetails> {
        let token = self.auth.installation_token().await?;

        let response = self
            .http
            .get(format!(
                "{}/repos/{owner}/{repo}/pulls/{number}",
                self.api_base_url
            ))
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(token)
            .send()
            .await
            .context("fetching GitHub pull request")?;
        into_api_result(response, "GitHub pull request lookup failed")
            .await?
            .json()
            .await
            .context("decoding GitHub pull request response")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestDetails {
    pub state: String,
    pub head: PullRequestRef,
    pub base: PullRequestBaseRef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestRef {
    #[serde(rename = "ref")]
    pub reference: String,
    pub sha: String,
    pub repo: PullRequestRepo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestBaseRef {
    #[serde(rename = "ref")]
    pub reference: String,
    pub repo: PullRequestRepo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestRepo {
    pub full_name: String,
}
