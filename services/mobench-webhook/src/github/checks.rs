use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;

use crate::github::{auth::GitHubAppAuth, into_api_result};

#[derive(Clone)]
pub struct GitHubChecksClient {
    auth: GitHubAppAuth,
    http: Client,
    api_base_url: String,
}

impl GitHubChecksClient {
    pub fn new(auth: GitHubAppAuth, api_base_url: impl Into<String>) -> Self {
        Self {
            auth,
            http: Client::new(),
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub async fn create_or_update_check_run(
        &self,
        owner: &str,
        repo: &str,
        check_run_id: Option<i64>,
        payload: &Value,
    ) -> Result<Value> {
        let token = self.auth.installation_token().await?;
        let request = match check_run_id {
            Some(check_run_id) => self.http.patch(format!(
                "{}/repos/{owner}/{repo}/check-runs/{check_run_id}",
                self.api_base_url
            )),
            None => self.http.post(format!(
                "{}/repos/{owner}/{repo}/check-runs",
                self.api_base_url
            )),
        };
        let response = request
            .header("Accept", "application/vnd.github+json")
            .bearer_auth(token)
            .json(payload)
            .send()
            .await
            .context("sending GitHub check run request")?;
        let response = into_api_result(response, "GitHub check run request failed").await?;

        response
            .json()
            .await
            .context("decoding GitHub check run response")
    }
}
