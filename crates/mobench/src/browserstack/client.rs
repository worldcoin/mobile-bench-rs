//! Authenticated BrowserStack HTTP transport.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use serde_json::Value;

use super::{
    BrowserStackAuth, BrowserStackClient, DEFAULT_BASE_URL, USER_AGENT, parse_response,
    should_authenticate_asset_url,
};

impl BrowserStackClient {
    pub fn new(auth: BrowserStackAuth, project: Option<String>) -> Result<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            // Artifact uploads may legitimately outlive Reqwest's default
            // request timeout. Provider polling has independent deadlines.
            .timeout(None)
            .connect_timeout(Duration::from_secs(15))
            // BrowserStack's large multipart uploads are reliable over HTTP/1.1.
            .http1_only()
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            http,
            auth,
            base_url: DEFAULT_BASE_URL.to_owned(),
            project,
        })
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub(super) fn api(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    pub fn get_json(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .get(self.api(path))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .send()
            .with_context(|| format!("requesting BrowserStack API {path}"))?;
        parse_response(response, path)
    }

    pub fn download_url(&self, url: &str, destination: &Path) -> Result<()> {
        let response = self
            .asset_request(url)
            .send()
            .with_context(|| format!("downloading BrowserStack asset {url}"))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .with_context(|| format!("reading BrowserStack asset body {url}"))?;
        if !status.is_success() {
            return Err(anyhow!(
                "BrowserStack asset download failed (status {status}): {}",
                String::from_utf8_lossy(&bytes)
            ));
        }
        std::fs::write(destination, bytes)
            .with_context(|| format!("writing BrowserStack asset to {destination:?}"))
    }

    pub fn get_device_logs(
        &self,
        build_id: &str,
        session_id: &str,
        platform: &str,
    ) -> Result<String> {
        let path = match platform {
            "espresso" => format!(
                "app-automate/espresso/v2/builds/{build_id}/sessions/{session_id}/devicelogs"
            ),
            "xcuitest" => format!(
                "app-automate/xcuitest/v2/builds/{build_id}/sessions/{session_id}/devicelogs"
            ),
            _ => return Err(anyhow!("unsupported platform: {platform}")),
        };
        let response = self
            .http
            .get(self.api(&path))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .send()
            .with_context(|| format!("fetching device logs for session {session_id}"))?;
        let status = response.status();
        let text = response.text().context("reading device logs response")?;
        if !status.is_success() {
            return Err(anyhow!(
                "Failed to fetch device logs (status {status}): {text}"
            ));
        }
        Ok(text)
    }

    pub(super) fn get_session_json(
        &self,
        build_id: &str,
        session_id: &str,
        platform: &str,
    ) -> Result<Value> {
        let path = match platform {
            "espresso" => {
                format!("app-automate/espresso/v2/builds/{build_id}/sessions/{session_id}")
            }
            "xcuitest" => {
                format!("app-automate/xcuitest/v2/builds/{build_id}/sessions/{session_id}")
            }
            _ => return Err(anyhow!("unsupported platform: {platform}")),
        };
        self.get_json(&path)
    }

    pub(super) fn download_text_url(&self, url: &str) -> Result<String> {
        let response = self
            .asset_request(url)
            .send()
            .with_context(|| format!("downloading BrowserStack asset {url}"))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .with_context(|| format!("reading BrowserStack asset body {url}"))?;
        if !status.is_success() {
            return Err(anyhow!(
                "BrowserStack asset download failed (status {status}): {}",
                String::from_utf8_lossy(&bytes)
            ));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn asset_request(&self, url: &str) -> reqwest::blocking::RequestBuilder {
        let request = self.http.get(url);
        if should_authenticate_asset_url(url) {
            request.basic_auth(&self.auth.username, Some(&self.auth.access_key))
        } else {
            request
        }
    }
}
