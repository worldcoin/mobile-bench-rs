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
    #[allow(dead_code)] // Used in tests to verify URL construction
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
            // Specify the test method to run (required by BrowserStack for XCUITest)
            only_testing: Some(vec![
                "BenchRunnerUITests/BenchRunnerUITests/testLaunchShowsBenchmarkReport".to_string()
            ]),
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

    /// Get the status of an Espresso build
    pub fn get_espresso_build_status(&self, build_id: &str) -> Result<BuildStatus> {
        let path = format!("app-automate/espresso/v2/builds/{}", build_id);
        let json = self.get_json(&path)?;
        let response: BuildStatusResponse = serde_json::from_value(json)
            .context("parsing build status response")?;
        Ok(response.into())
    }

    /// Get the status of an XCUITest build
    pub fn get_xcuitest_build_status(&self, build_id: &str) -> Result<BuildStatus> {
        let path = format!("app-automate/xcuitest/v2/builds/{}", build_id);
        let json = self.get_json(&path)?;
        let response: BuildStatusResponse = serde_json::from_value(json)
            .context("parsing build status response")?;
        Ok(response.into())
    }

    /// Poll for build completion with timeout
    ///
    /// # Arguments
    /// * `build_id` - The build ID to poll
    /// * `platform` - "espresso" or "xcuitest"
    /// * `timeout_secs` - Maximum time to wait in seconds (default: 600)
    /// * `poll_interval_secs` - How often to check status in seconds (default: 10)
    pub fn poll_build_completion(
        &self,
        build_id: &str,
        platform: &str,
        timeout_secs: u64,
        poll_interval_secs: u64,
    ) -> Result<BuildStatus> {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let timeout = Duration::from_secs(timeout_secs);
        let poll_interval = Duration::from_secs(poll_interval_secs);

        loop {
            let status = match platform {
                "espresso" => self.get_espresso_build_status(build_id)?,
                "xcuitest" => self.get_xcuitest_build_status(build_id)?,
                _ => return Err(anyhow!("unsupported platform: {}", platform)),
            };

            match status.status.as_str() {
                "done" => return Ok(status),
                "failed" | "error" | "timeout" => {
                    return Err(anyhow!(
                        "Build {} failed with status: {}",
                        build_id,
                        status.status
                    ));
                }
                _ => {
                    // Still running
                    if start.elapsed() >= timeout {
                        return Err(anyhow!(
                            "Timeout waiting for build {} to complete (waited {} seconds)",
                            build_id,
                            timeout_secs
                        ));
                    }
                    std::thread::sleep(poll_interval);
                }
            }
        }
    }

    /// Fetch device logs for a specific session
    pub fn get_device_logs(&self, build_id: &str, session_id: &str, platform: &str) -> Result<String> {
        let path = match platform {
            "espresso" => format!(
                "app-automate/espresso/v2/builds/{}/sessions/{}/devicelogs",
                build_id, session_id
            ),
            "xcuitest" => format!(
                "app-automate/xcuitest/v2/builds/{}/sessions/{}/devicelogs",
                build_id, session_id
            ),
            _ => return Err(anyhow!("unsupported platform: {}", platform)),
        };

        let resp = self
            .http
            .get(self.api(&path))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .send()
            .with_context(|| format!("fetching device logs for session {}", session_id))?;

        let status = resp.status();
        let text = resp.text().context("reading device logs response")?;

        if !status.is_success() {
            return Err(anyhow!(
                "Failed to fetch device logs (status {}): {}",
                status,
                text
            ));
        }

        Ok(text)
    }

    /// Extract benchmark results from device logs
    /// Looks for JSON output matching BenchReport format
    pub fn extract_benchmark_results(&self, logs: &str) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        // Look for JSON objects that contain benchmark-related fields
        for line in logs.lines() {
            let trimmed = line.trim();
            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.contains("\"function\"") && trimmed.contains("\"samples\""))
            {
                if let Ok(json) = serde_json::from_str::<Value>(trimmed) {
                    // Check if this looks like a benchmark report
                    if json.get("function").is_some() || json.get("samples").is_some() {
                        results.push(json);
                    }
                }
            }
        }

        if results.is_empty() {
            Err(anyhow!("No benchmark results found in device logs"))
        } else {
            Ok(results)
        }
    }

    /// Extract performance metrics from device logs
    /// Looks for JSON objects with "type":"performance" or similar performance indicators
    pub fn extract_performance_metrics(&self, logs: &str) -> Result<PerformanceMetrics> {
        let mut snapshots = Vec::new();

        for line in logs.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                if let Ok(json) = serde_json::from_str::<Value>(trimmed) {
                    // Check if this looks like a performance metric
                    if json.get("type").and_then(|t| t.as_str()) == Some("performance")
                        || json.get("memory").is_some()
                        || json.get("cpu").is_some()
                    {
                        if let Ok(snapshot) = serde_json::from_value::<PerformanceSnapshot>(json) {
                            snapshots.push(snapshot);
                        }
                    }
                }
            }
        }

        Ok(PerformanceMetrics::from_snapshots(snapshots))
    }

    /// Wait for build completion and fetch all benchmark results
    ///
    /// This is a convenience method that:
    /// 1. Polls for build completion (with timeout)
    /// 2. Fetches device logs for all sessions
    /// 3. Extracts benchmark results from logs
    ///
    /// # Arguments
    /// * `build_id` - The build ID returned from schedule_*_run
    /// * `platform` - "espresso" or "xcuitest"
    /// * `timeout_secs` - Maximum time to wait (default: 600)
    ///
    /// # Returns
    /// A map of device names to their benchmark results
    pub fn wait_and_fetch_results(
        &self,
        build_id: &str,
        platform: &str,
        timeout_secs: Option<u64>,
    ) -> Result<std::collections::HashMap<String, Vec<Value>>> {
        let timeout = timeout_secs.unwrap_or(600);

        println!("Waiting for build {} to complete (timeout: {}s)...", build_id, timeout);
        let build_status = self.poll_build_completion(build_id, platform, timeout, 10)?;

        println!("Build completed with status: {}", build_status.status);
        println!("Fetching results from {} device(s)...", build_status.devices.len());

        let mut results = std::collections::HashMap::new();

        for device in &build_status.devices {
            println!("  Fetching logs for {} (session: {})...", device.device, device.session_id);

            match self.get_device_logs(build_id, &device.session_id, platform) {
                Ok(logs) => {
                    match self.extract_benchmark_results(&logs) {
                        Ok(bench_results) => {
                            println!("    Found {} benchmark result(s)", bench_results.len());
                            results.insert(device.device.clone(), bench_results);
                        }
                        Err(e) => {
                            println!("    Warning: {}", e);
                        }
                    }
                }
                Err(e) => {
                    println!("    Failed to fetch logs: {}", e);
                }
            }
        }

        if results.is_empty() {
            Err(anyhow!("No benchmark results found from any device"))
        } else {
            Ok(results)
        }
    }

    /// Wait for build completion and fetch all results including performance metrics
    ///
    /// Returns both benchmark results and performance metrics
    pub fn wait_and_fetch_all_results(
        &self,
        build_id: &str,
        platform: &str,
        timeout_secs: Option<u64>,
    ) -> Result<(
        std::collections::HashMap<String, Vec<Value>>,
        std::collections::HashMap<String, PerformanceMetrics>,
    )> {
        let timeout = timeout_secs.unwrap_or(600);

        println!("Waiting for build {} to complete (timeout: {}s)...", build_id, timeout);
        let build_status = self.poll_build_completion(build_id, platform, timeout, 10)?;

        println!("Build completed with status: {}", build_status.status);
        println!("Fetching results from {} device(s)...", build_status.devices.len());

        let mut benchmark_results = std::collections::HashMap::new();
        let mut performance_metrics = std::collections::HashMap::new();

        for device in &build_status.devices {
            println!("  Fetching logs for {} (session: {})...", device.device, device.session_id);

            match self.get_device_logs(build_id, &device.session_id, platform) {
                Ok(logs) => {
                    // Extract benchmark results
                    match self.extract_benchmark_results(&logs) {
                        Ok(bench_results) => {
                            println!("    Found {} benchmark result(s)", bench_results.len());
                            benchmark_results.insert(device.device.clone(), bench_results);
                        }
                        Err(e) => {
                            println!("    Warning: No benchmark results - {}", e);
                        }
                    }

                    // Extract performance metrics
                    match self.extract_performance_metrics(&logs) {
                        Ok(perf_metrics) if perf_metrics.sample_count > 0 => {
                            println!("    Found {} performance metric snapshot(s)", perf_metrics.sample_count);
                            performance_metrics.insert(device.device.clone(), perf_metrics);
                        }
                        Ok(_) => {
                            println!("    No performance metrics found");
                        }
                        Err(e) => {
                            println!("    Warning: Failed to extract performance metrics - {}", e);
                        }
                    }
                }
                Err(e) => {
                    println!("    Failed to fetch logs: {}", e);
                }
            }
        }

        if benchmark_results.is_empty() {
            Err(anyhow!("No benchmark results found from any device"))
        } else {
            Ok((benchmark_results, performance_metrics))
        }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildStatus {
    pub build_id: String,
    pub status: String,
    pub duration: Option<u64>,
    pub devices: Vec<DeviceSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceSnapshot {
    #[serde(default)]
    pub timestamp_ms: Option<u64>,
    #[serde(flatten)]
    pub metrics: PerformanceData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<CpuMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetrics {
    #[serde(alias = "used_mb", alias = "usedMb")]
    pub used_mb: Option<f64>,
    #[serde(alias = "max_mb", alias = "maxMb")]
    pub max_mb: Option<f64>,
    #[serde(alias = "available_mb", alias = "availableMb")]
    pub available_mb: Option<f64>,
    #[serde(alias = "total_mb", alias = "totalMb")]
    pub total_mb: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuMetrics {
    #[serde(alias = "usage_percent", alias = "usagePercent")]
    pub usage_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PerformanceMetrics {
    pub sample_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<AggregateMemoryMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<AggregateCpuMetrics>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub snapshots: Vec<PerformanceSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateMemoryMetrics {
    pub peak_mb: f64,
    pub average_mb: f64,
    pub min_mb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateCpuMetrics {
    pub peak_percent: f64,
    pub average_percent: f64,
    pub min_percent: f64,
}

impl PerformanceMetrics {
    pub fn from_snapshots(snapshots: Vec<PerformanceSnapshot>) -> Self {
        if snapshots.is_empty() {
            return Self::default();
        }

        let sample_count = snapshots.len();

        // Aggregate memory metrics
        let memory_values: Vec<f64> = snapshots
            .iter()
            .filter_map(|s| s.metrics.memory.as_ref()?.used_mb)
            .collect();

        let memory = if !memory_values.is_empty() {
            Some(AggregateMemoryMetrics {
                peak_mb: memory_values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b)),
                average_mb: memory_values.iter().sum::<f64>() / memory_values.len() as f64,
                min_mb: memory_values.iter().fold(f64::INFINITY, |a, &b| a.min(b)),
            })
        } else {
            None
        };

        // Aggregate CPU metrics
        let cpu_values: Vec<f64> = snapshots
            .iter()
            .filter_map(|s| s.metrics.cpu.as_ref()?.usage_percent)
            .collect();

        let cpu = if !cpu_values.is_empty() {
            Some(AggregateCpuMetrics {
                peak_percent: cpu_values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b)),
                average_percent: cpu_values.iter().sum::<f64>() / cpu_values.len() as f64,
                min_percent: cpu_values.iter().fold(f64::INFINITY, |a, &b| a.min(b)),
            })
        } else {
            None
        };

        Self {
            sample_count,
            memory,
            cpu,
            snapshots,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSession {
    pub device: String,
    #[serde(alias = "sessionId", alias = "session_id")]
    pub session_id: String,
    pub status: String,
    #[serde(alias = "deviceLogs", alias = "device_logs")]
    pub device_logs: Option<String>,
}

// Internal response format from BrowserStack API
#[derive(Debug, Deserialize)]
struct BuildStatusResponse {
    #[serde(alias = "buildId", alias = "build_id")]
    build_id: String,
    status: String,
    duration: Option<u64>,
    devices: Option<Vec<DeviceSessionResponse>>,
}

#[derive(Debug, Deserialize)]
struct DeviceSessionResponse {
    device: String,
    #[serde(alias = "sessionId", alias = "session_id", alias = "hashed_id")]
    session_id: String,
    status: String,
    #[serde(alias = "deviceLogs", alias = "device_logs")]
    device_logs: Option<String>,
}

impl From<BuildStatusResponse> for BuildStatus {
    fn from(resp: BuildStatusResponse) -> Self {
        BuildStatus {
            build_id: resp.build_id,
            status: resp.status,
            duration: resp.duration,
            devices: resp
                .devices
                .unwrap_or_default()
                .into_iter()
                .map(|d| DeviceSession {
                    device: d.device,
                    session_id: d.session_id,
                    status: d.status,
                    device_logs: d.device_logs,
                })
                .collect(),
        }
    }
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
    #[serde(rename = "only-testing", skip_serializing_if = "Option::is_none")]
    only_testing: Option<Vec<String>>,
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

    #[test]
    fn suppresses_dead_code_warning_for_test_helper() {
        // This test uses with_base_url to verify it works and suppress the warning
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap()
        .with_base_url("https://test.example.com");

        assert_eq!(client.base_url, "https://test.example.com");
    }

    #[test]
    fn new_client_uses_default_base_url() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "testuser".into(),
                access_key: "testkey".into(),
            },
            Some("test-project".into()),
        )
        .unwrap();

        assert_eq!(client.base_url, DEFAULT_BASE_URL);
        assert_eq!(client.project, Some("test-project".to_string()));
    }

    #[test]
    fn api_constructs_url_correctly() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let url = client.api("app-automate/espresso/v2/app");
        assert_eq!(
            url,
            "https://api-cloud.browserstack.com/app-automate/espresso/v2/app"
        );
    }

    #[test]
    fn api_handles_leading_slash() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let url = client.api("/app-automate/builds");
        assert_eq!(
            url,
            "https://api-cloud.browserstack.com/app-automate/builds"
        );
    }

    #[test]
    fn api_handles_trailing_slash_in_base_url() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap()
        .with_base_url("https://test.example.com/");

        let url = client.api("endpoint");
        assert_eq!(url, "https://test.example.com/endpoint");
    }

    #[test]
    fn schedule_espresso_run_rejects_empty_devices() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let result = client.schedule_espresso_run(&[], "bs://app123", "bs://test456");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn schedule_espresso_run_rejects_empty_app_url() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let result = client.schedule_espresso_run(&["Pixel 7-13".to_string()], "", "bs://test456");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("app_url"));
    }

    #[test]
    fn schedule_espresso_run_rejects_empty_test_suite_url() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let result = client.schedule_espresso_run(&["Pixel 7-13".to_string()], "bs://app123", "");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("test_suite_url"));
    }

    #[test]
    fn schedule_xcuitest_run_rejects_empty_devices() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let result = client.schedule_xcuitest_run(&[], "bs://app123", "bs://test456");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn upload_xcuitest_app_rejects_missing_artifact() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let missing = Path::new("/tmp/nonexistent-ios-app.ipa");
        assert!(client.upload_xcuitest_app(missing).is_err());
    }

    #[test]
    fn upload_xcuitest_test_suite_rejects_missing_artifact() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let missing = Path::new("/tmp/nonexistent-test-suite.zip");
        assert!(client.upload_xcuitest_test_suite(missing).is_err());
    }

    #[test]
    fn extract_benchmark_results_finds_json_in_logs() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
Some device output
2026-01-14 12:00:00 Starting test
{"function": "sample_fns::fibonacci", "samples": [{"duration_ns": 1000}, {"duration_ns": 1200}], "mean_ns": 1100}
More output here
Test completed
        "#;

        let results = client.extract_benchmark_results(logs).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].get("function").unwrap().as_str().unwrap(), "sample_fns::fibonacci");
        assert_eq!(results[0].get("mean_ns").unwrap().as_u64().unwrap(), 1100);
    }

    #[test]
    fn extract_benchmark_results_handles_multiple_results() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
{"function": "test1", "samples": [{"duration_ns": 1000}]}
Some other output
{"function": "test2", "samples": [{"duration_ns": 2000}]}
        "#;

        let results = client.extract_benchmark_results(logs).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].get("function").unwrap().as_str().unwrap(), "test1");
        assert_eq!(results[1].get("function").unwrap().as_str().unwrap(), "test2");
    }

    #[test]
    fn extract_benchmark_results_returns_error_when_no_results() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
Just some regular logs
No benchmark data here
Test completed
        "#;

        let result = client.extract_benchmark_results(logs);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No benchmark results"));
    }

    #[test]
    fn extract_benchmark_results_ignores_invalid_json() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
{"invalid": "json without function or samples"}
{"function": "test1", "samples": [{"duration_ns": 1000}]}
{broken json}
        "#;

        let results = client.extract_benchmark_results(logs).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].get("function").unwrap().as_str().unwrap(), "test1");
    }

    #[test]
    fn build_status_conversion_from_response() {
        let response = BuildStatusResponse {
            build_id: "test123".to_string(),
            status: "done".to_string(),
            duration: Some(120),
            devices: Some(vec![
                DeviceSessionResponse {
                    device: "Pixel 7-13".to_string(),
                    session_id: "session123".to_string(),
                    status: "passed".to_string(),
                    device_logs: Some("https://example.com/logs".to_string()),
                },
            ]),
        };

        let status: BuildStatus = response.into();
        assert_eq!(status.build_id, "test123");
        assert_eq!(status.status, "done");
        assert_eq!(status.duration, Some(120));
        assert_eq!(status.devices.len(), 1);
        assert_eq!(status.devices[0].device, "Pixel 7-13");
        assert_eq!(status.devices[0].session_id, "session123");
    }

    #[test]
    fn build_status_conversion_handles_missing_devices() {
        let response = BuildStatusResponse {
            build_id: "test456".to_string(),
            status: "running".to_string(),
            duration: None,
            devices: None,
        };

        let status: BuildStatus = response.into();
        assert_eq!(status.build_id, "test456");
        assert_eq!(status.status, "running");
        assert_eq!(status.devices.len(), 0);
    }

    #[test]
    fn device_session_deserializes_from_json() {
        let json = r#"{
            "device": "iPhone 14-16",
            "sessionId": "abc123",
            "status": "passed",
            "deviceLogs": "https://example.com/logs"
        }"#;

        let session: DeviceSessionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(session.device, "iPhone 14-16");
        assert_eq!(session.session_id, "abc123");
        assert_eq!(session.status, "passed");
    }

    #[test]
    fn device_session_handles_alternative_field_names() {
        let json = r#"{
            "device": "Pixel 7",
            "hashed_id": "xyz789",
            "status": "running",
            "device_logs": "https://example.com/logs"
        }"#;

        let session: DeviceSessionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(session.device, "Pixel 7");
        assert_eq!(session.session_id, "xyz789");
    }

    #[test]
    fn extract_performance_metrics_finds_memory_and_cpu() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
Some device output
2026-01-14 12:00:00 Starting test
{"type": "performance", "timestamp_ms": 1705238400000, "memory": {"used_mb": 128.5, "max_mb": 512.0}, "cpu": {"usage_percent": 45.2}}
{"type": "performance", "timestamp_ms": 1705238401000, "memory": {"used_mb": 135.0, "max_mb": 512.0}, "cpu": {"usage_percent": 52.1}}
More output here
        "#;

        let metrics = client.extract_performance_metrics(logs).unwrap();
        assert_eq!(metrics.sample_count, 2);

        assert!(metrics.memory.is_some());
        let mem = metrics.memory.as_ref().unwrap();
        assert_eq!(mem.peak_mb, 135.0);
        assert_eq!(mem.average_mb, 131.75); // (128.5 + 135.0) / 2
        assert_eq!(mem.min_mb, 128.5);

        assert!(metrics.cpu.is_some());
        let cpu = metrics.cpu.as_ref().unwrap();
        assert_eq!(cpu.peak_percent, 52.1);
        assert!((cpu.average_percent - 48.65).abs() < 0.001); // (45.2 + 52.1) / 2
        assert_eq!(cpu.min_percent, 45.2);
    }

    #[test]
    fn extract_performance_metrics_handles_memory_only() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
{"memory": {"used_mb": 100.0, "max_mb": 512.0}}
{"memory": {"used_mb": 120.0, "max_mb": 512.0}}
        "#;

        let metrics = client.extract_performance_metrics(logs).unwrap();
        assert_eq!(metrics.sample_count, 2);
        assert!(metrics.memory.is_some());
        assert!(metrics.cpu.is_none());

        let mem = metrics.memory.as_ref().unwrap();
        assert_eq!(mem.peak_mb, 120.0);
        assert_eq!(mem.average_mb, 110.0);
    }

    #[test]
    fn extract_performance_metrics_handles_cpu_only() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
{"cpu": {"usage_percent": 30.5}}
{"cpu": {"usage_percent": 40.5}}
{"cpu": {"usage_percent": 35.0}}
        "#;

        let metrics = client.extract_performance_metrics(logs).unwrap();
        assert_eq!(metrics.sample_count, 3);
        assert!(metrics.memory.is_none());
        assert!(metrics.cpu.is_some());

        let cpu = metrics.cpu.as_ref().unwrap();
        assert_eq!(cpu.peak_percent, 40.5);
        assert_eq!(cpu.min_percent, 30.5);
        // Average: (30.5 + 40.5 + 35.0) / 3 = 35.333...
        assert!((cpu.average_percent - 35.333333).abs() < 0.001);
    }

    #[test]
    fn extract_performance_metrics_returns_empty_when_no_metrics() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
Just some regular logs
No performance data here
Test completed
        "#;

        let metrics = client.extract_performance_metrics(logs).unwrap();
        assert_eq!(metrics.sample_count, 0);
        assert!(metrics.memory.is_none());
        assert!(metrics.cpu.is_none());
    }

    #[test]
    fn extract_performance_metrics_ignores_invalid_json() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
{"invalid": "json without performance fields"}
{"memory": {"used_mb": 100.0}}
{broken json}
{"cpu": {"usage_percent": 50.0}}
        "#;

        let metrics = client.extract_performance_metrics(logs).unwrap();
        assert_eq!(metrics.sample_count, 2);
        assert!(metrics.memory.is_some());
        assert!(metrics.cpu.is_some());
    }

    #[test]
    fn extract_performance_metrics_handles_alternative_field_names() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        // Test camelCase variants
        let logs = r#"
{"memory": {"usedMb": 128.5, "maxMb": 512.0, "availableMb": 383.5}}
        "#;

        let metrics = client.extract_performance_metrics(logs).unwrap();
        assert_eq!(metrics.sample_count, 1);

        let mem = metrics.memory.as_ref().unwrap();
        assert_eq!(mem.peak_mb, 128.5);
    }

    #[test]
    fn performance_metrics_aggregates_correctly_with_mixed_data() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
{"memory": {"used_mb": 100.0}}
{"cpu": {"usage_percent": 30.0}}
{"memory": {"used_mb": 150.0}, "cpu": {"usage_percent": 50.0}}
        "#;

        let metrics = client.extract_performance_metrics(logs).unwrap();
        assert_eq!(metrics.sample_count, 3);

        // Memory should aggregate from snapshots 1 and 3
        let mem = metrics.memory.as_ref().unwrap();
        assert_eq!(mem.peak_mb, 150.0);
        assert_eq!(mem.min_mb, 100.0);
        assert_eq!(mem.average_mb, 125.0); // (100 + 150) / 2

        // CPU should aggregate from snapshots 2 and 3
        let cpu = metrics.cpu.as_ref().unwrap();
        assert_eq!(cpu.peak_percent, 50.0);
        assert_eq!(cpu.min_percent, 30.0);
        assert_eq!(cpu.average_percent, 40.0); // (30 + 50) / 2
    }
}
