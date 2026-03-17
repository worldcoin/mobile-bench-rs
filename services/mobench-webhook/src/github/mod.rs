pub mod artifacts;
pub mod auth;
pub mod checks;
pub mod pull_requests;
pub mod workflows;

use std::{error::Error as StdError, fmt};

use anyhow::{Context, Result, anyhow};
use reqwest::{Client, Response, StatusCode};
use time::OffsetDateTime;

pub const USER_AGENT: &str = "mobench-webhook";

#[derive(Clone)]
pub struct GitHubClients {
    pub workflows: workflows::GitHubWorkflowsClient,
    pub artifacts: artifacts::GitHubArtifactsClient,
    pub checks: checks::GitHubChecksClient,
    pub pull_requests: pull_requests::GitHubPullRequestsClient,
}

impl GitHubClients {
    pub fn new(auth: auth::GitHubAppAuth, api_base_url: impl Into<String>) -> Self {
        let api_base_url = api_base_url.into();

        Self {
            workflows: workflows::GitHubWorkflowsClient::new(auth.clone(), api_base_url.clone()),
            artifacts: artifacts::GitHubArtifactsClient::new(auth.clone(), api_base_url.clone()),
            checks: checks::GitHubChecksClient::new(auth.clone(), api_base_url.clone()),
            pull_requests: pull_requests::GitHubPullRequestsClient::new(auth, api_base_url),
        }
    }
}

pub fn api_http_client() -> Client {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("GitHub API HTTP client should initialize")
}

#[derive(Debug)]
pub struct GitHubRateLimitError {
    retry_after_secs: i32,
    status: StatusCode,
    body: String,
}

impl GitHubRateLimitError {
    pub fn retry_after_secs(&self) -> i32 {
        self.retry_after_secs
    }
}

impl fmt::Display for GitHubRateLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.body.is_empty() {
            write!(
                f,
                "GitHub rate limit exceeded (status {}, retry after {}s)",
                self.status.as_u16(),
                self.retry_after_secs
            )
        } else {
            write!(
                f,
                "GitHub rate limit exceeded (status {}, retry after {}s): {}",
                self.status.as_u16(),
                self.retry_after_secs,
                self.body
            )
        }
    }
}

impl StdError for GitHubRateLimitError {}

pub async fn into_api_result(response: Response, context: &'static str) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let retry_after_secs = rate_limit_retry_after_seconds(&response);
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| String::new())
        .trim()
        .to_string();

    if let Some(retry_after_secs) = retry_after_secs {
        return Err(anyhow!(GitHubRateLimitError {
            retry_after_secs,
            status,
            body,
        }))
        .context(context);
    }

    if body.is_empty() {
        Err(anyhow!("{} (status {})", context, status.as_u16()))
    } else {
        Err(anyhow!("{} (status {}): {}", context, status.as_u16(), body))
    }
}

pub fn find_rate_limit_retry_after(err: &anyhow::Error) -> Option<i32> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<GitHubRateLimitError>())
        .map(GitHubRateLimitError::retry_after_secs)
}

fn rate_limit_retry_after_seconds(response: &Response) -> Option<i32> {
    let status = response.status();
    let headers = response.headers();
    let looks_rate_limited = status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && headers
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                == Some("0"));
    if !looks_rate_limited {
        return None;
    }

    if let Some(retry_after) = headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i32>().ok())
    {
        return Some(retry_after.max(1));
    }

    headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .map(|reset_at| {
            let now = OffsetDateTime::now_utc().unix_timestamp();
            (reset_at - now).clamp(1, i64::from(i32::MAX)) as i32
        })
}
