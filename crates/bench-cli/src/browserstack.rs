use anyhow::{Context, Result, anyhow};
use reqwest::blocking::multipart::Form;
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::path::Path;

const DEFAULT_BASE_URL: &str = "https://api-cloud.browserstack.com";
const USER_AGENT: &str = "mobile-bench-rs/0.1";

#[derive(Debug, Clone)]
pub struct BrowserStackAuth {
    pub username: String,
    pub access_key: String,
}

/// BrowserStack App Automate (Espresso) client.
#[derive(Debug, Clone)]
pub struct BrowserStackClient {
    http: Client,
    auth: BrowserStackAuth,
    base_url: String,
    project: Option<String>,
}

impl BrowserStackClient {
    pub fn new(auth: BrowserStackAuth, project: Option<String>) -> Result<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .context("building HTTP client")?;

        Ok(Self {
            http,
            auth,
            base_url: DEFAULT_BASE_URL.to_string(),
            project,
        })
    }

    #[cfg(test)]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Upload an Espresso app-under-test APK to BrowserStack.
    pub fn upload_espresso_app(&self, artifact: &Path) -> Result<AppUpload> {
        if !artifact.exists() {
            return Err(anyhow!("app artifact not found at {:?}", artifact));
        }

        let form = Form::new().file("file", artifact)?;
        let resp = self
            .http
            .post(self.api("app-automate/espresso/v2/app"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(form)
            .send()
            .context("uploading app to BrowserStack")?;

        parse_response(resp, "app upload")
    }

    /// Upload an Espresso test-suite APK to BrowserStack.
    pub fn upload_espresso_test_suite(&self, artifact: &Path) -> Result<TestSuiteUpload> {
        if !artifact.exists() {
            return Err(anyhow!("test suite artifact not found at {:?}", artifact));
        }

        let form = Form::new().file("file", artifact)?;
        let resp = self
            .http
            .post(self.api("app-automate/espresso/v2/test-suite"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(form)
            .send()
            .context("uploading test suite to BrowserStack")?;

        parse_response(resp, "test suite upload")
    }

    pub fn upload_xcuitest_app(&self, artifact: &Path) -> Result<AppUpload> {
        if !artifact.exists() {
            return Err(anyhow!("iOS app artifact not found at {:?}", artifact));
        }

        let form = Form::new().file("file", artifact)?;
        let resp = self
            .http
            .post(self.api("app-automate/xcuitest/v2/app"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(form)
            .send()
            .context("uploading iOS app to BrowserStack")?;

        parse_response(resp, "iOS app upload")
    }

    pub fn upload_xcuitest_test_suite(&self, artifact: &Path) -> Result<TestSuiteUpload> {
        if !artifact.exists() {
            return Err(anyhow!(
                "iOS XCUITest suite artifact not found at {:?}",
                artifact
            ));
        }

        let form = Form::new().file("file", artifact)?;
        let resp = self
            .http
            .post(self.api("app-automate/xcuitest/v2/test-suite"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(form)
            .send()
            .context("uploading iOS XCUITest suite to BrowserStack")?;

        parse_response(resp, "iOS XCUITest suite upload")
    }

    pub fn schedule_espresso_run(
        &self,
        devices: &[String],
        app_url: &str,
        test_suite_url: &str,
    ) -> Result<ScheduledRun> {
        if devices.is_empty() {
            return Err(anyhow!("device list is empty; provide at least one target"));
        }
        if app_url.is_empty() {
            return Err(anyhow!("app_url is empty"));
        }
        if test_suite_url.is_empty() {
            return Err(anyhow!("test_suite_url is empty"));
        }

        let body = BuildRequest {
            app: app_url.to_string(),
            test_suite: test_suite_url.to_string(),
            devices: devices.to_vec(),
            device_logs: true,
            disable_animations: true,
            build_name: self.project.clone(),
        };

        let resp = self
            .http
            .post(self.api("app-automate/espresso/v2/build"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .json(&body)
            .send()
            .context("scheduling BrowserStack Espresso run")?;

        let build: BuildResponse = parse_response(resp, "schedule run")?;
        Ok(ScheduledRun {
            build_id: build.build_id,
        })
    }

    pub fn schedule_xcuitest_run(
        &self,
        devices: &[String],
        app_url: &str,
        test_suite_url: &str,
    ) -> Result<ScheduledRun> {
        if devices.is_empty() {
            return Err(anyhow!("device list is empty; provide at least one target"));
        }
        if app_url.is_empty() {
            return Err(anyhow!("app_url is empty"));
        }
        if test_suite_url.is_empty() {
            return Err(anyhow!("test_suite_url is empty"));
        }

        let body = XcuitestBuildRequest {
            app: app_url.to_string(),
            test_suite: test_suite_url.to_string(),
            devices: devices.to_vec(),
            device_logs: true,
            build_name: self.project.clone(),
        };

        let resp = self
            .http
            .post(self.api("app-automate/xcuitest/v2/build"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .json(&body)
            .send()
            .context("scheduling BrowserStack XCUITest run")?;

        let build: BuildResponse = parse_response(resp, "schedule run")?;
        Ok(ScheduledRun {
            build_id: build.build_id,
        })
    }

    fn api(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    pub fn get_json(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .get(self.api(path))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .send()
            .with_context(|| format!("requesting BrowserStack API {}", path))?;

        parse_response(resp, path)
    }

    pub fn download_url(&self, url: &str, dest: &Path) -> Result<()> {
        let resp = self
            .http
            .get(url)
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .send()
            .with_context(|| format!("downloading BrowserStack asset {}", url))?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .with_context(|| format!("reading BrowserStack asset body {}", url))?;
        if !status.is_success() {
            return Err(anyhow!(
                "BrowserStack asset download failed (status {}): {}",
                status,
                String::from_utf8_lossy(&bytes)
            ));
        }
        std::fs::write(dest, bytes)
            .with_context(|| format!("writing BrowserStack asset to {:?}", dest))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppUpload {
    #[serde(alias = "appUrl")]
    pub app_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestSuiteUpload {
    #[serde(alias = "test_suite_url", alias = "testSuiteUrl")]
    pub test_suite_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduledRun {
    pub build_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildRequest {
    app: String,
    test_suite: String,
    devices: Vec<String>,
    device_logs: bool,
    disable_animations: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct XcuitestBuildRequest {
    app: String,
    test_suite: String,
    devices: Vec<String>,
    device_logs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BuildResponse {
    #[serde(alias = "build_id", alias = "buildId")]
    build_id: String,
}

fn parse_response<T: DeserializeOwned>(resp: Response, context: &str) -> Result<T> {
    let status = resp.status();
    let text = resp
        .text()
        .with_context(|| format!("reading BrowserStack API response body for {}", context))?;

    if !status.is_success() {
        return Err(anyhow!(
            "BrowserStack API {} failed (status {}): {}",
            context,
            status,
            text
        ));
    }

    serde_json::from_str(&text)
        .with_context(|| format!("parsing BrowserStack API response for {}", context))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_artifact() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();
        let missing = Path::new("/tmp/definitely-missing-file");
        assert!(client.upload_espresso_app(missing).is_err());
    }
}
