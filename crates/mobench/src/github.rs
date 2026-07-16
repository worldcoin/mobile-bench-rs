//! GitHub Checks API client for creating Check Runs.

use anyhow::{Context, Result};
pub use mobench_report::CheckRunAnnotation;
use mobench_report::{CheckRunRequest, GITHUB_CHECK_ANNOTATION_LIMIT};

const GITHUB_API_BASE: &str = "https://api.github.com";

pub struct GitHubClient {
    http: reqwest::blocking::Client,
    token: String,
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
        annotations: Vec<CheckRunAnnotation>,
    ) -> Result<CheckRunResult> {
        let url = format!("{GITHUB_API_BASE}/repos/{repo}/check-runs");
        if annotations.len() > GITHUB_CHECK_ANNOTATION_LIMIT {
            eprintln!(
                "Warning: {} annotations exceed GitHub's {}-annotation limit, truncating",
                annotations.len(),
                GITHUB_CHECK_ANNOTATION_LIMIT,
            );
        }
        let body = CheckRunRequest::completed(name, sha, conclusion, title, summary, annotations);
        let annotations_count = body.annotations_count();

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
            conclusion: body.conclusion,
            annotations_count,
        })
    }
}
