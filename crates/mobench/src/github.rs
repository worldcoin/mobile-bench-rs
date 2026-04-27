//! GitHub Checks API client for creating Check Runs.

use anyhow::{Context, Result};
use serde::Serialize;

const GITHUB_API_BASE: &str = "https://api.github.com";

pub struct GitHubClient {
    http: reqwest::blocking::Client,
    token: String,
}

#[derive(Debug, Serialize)]
struct CreateCheckRunRequest {
    name: String,
    head_sha: String,
    status: String,
    conclusion: String,
    output: CheckRunOutput,
}

#[derive(Debug, Serialize)]
struct CheckRunOutput {
    title: String,
    summary: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    annotations: Vec<CheckRunAnnotation>,
}

#[derive(Debug, Serialize)]
pub struct CheckRunAnnotation {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub annotation_level: String,
    pub message: String,
    pub title: String,
}

pub struct CheckRunResult {
    pub conclusion: String,
    pub annotations_count: usize,
}

impl GitHubClient {
    pub fn new(token: String) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .user_agent("mobench")
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { http, token })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_check_run(
        &self,
        repo: &str,
        sha: &str,
        name: &str,
        conclusion: &str,
        title: &str,
        summary: &str,
        mut annotations: Vec<CheckRunAnnotation>,
    ) -> Result<CheckRunResult> {
        let url = format!("{GITHUB_API_BASE}/repos/{repo}/check-runs");
        if annotations.len() > 50 {
            eprintln!(
                "Warning: {} annotations exceed GitHub's 50-annotation limit, truncating",
                annotations.len()
            );
            annotations.truncate(50);
        }
        let annotations_count = annotations.len();

        let body = CreateCheckRunRequest {
            name: name.to_string(),
            head_sha: sha.to_string(),
            status: "completed".to_string(),
            conclusion: conclusion.to_string(),
            output: CheckRunOutput {
                title: title.to_string(),
                summary: summary.to_string(),
                annotations,
            },
        };

        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&body)
            .send()
            .context("Failed to send Check Run request")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            anyhow::bail!("GitHub API returned {status}: {text}");
        }

        Ok(CheckRunResult {
            conclusion: conclusion.to_string(),
            annotations_count,
        })
    }
}
