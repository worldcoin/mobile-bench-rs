use anyhow::{Context, Result, anyhow};
use mobench_process::ProcessCancellation;
use mobench_provider::{
    AdapterRun, CollectedOutput, ExpectedSession as ProviderExpectedSession, ProviderRun,
};
use mobench_runtime::Distribution;
use reqwest::Url;
use reqwest::blocking::multipart::Form;
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::path::{Path, PathBuf};
use thiserror::Error;

mod adapter;
mod polling;
mod reconciliation;

pub(crate) use adapter::BrowserStackProviderAdapter;
#[cfg(test)]
use reconciliation::provider_device_identifier;
use reconciliation::{
    ReconciledDeviceSession, is_valid_device_selector, reconcile_requested_device_sessions,
};

type BrowserStackResults = (
    std::collections::HashMap<String, Vec<Value>>,
    std::collections::HashMap<String, PerformanceMetrics>,
);

#[derive(Clone, Debug)]
pub(crate) struct BrowserStackCollectedReport {
    pub(crate) requested_device_id: String,
    pub(crate) observed_device_id: String,
    pub(crate) transport_session_id: String,
    pub(crate) benchmark: Value,
}

#[derive(Debug)]
pub(crate) struct BrowserStackCollection {
    pub(crate) benchmark_results: std::collections::HashMap<String, Vec<Value>>,
    pub(crate) performance_metrics: std::collections::HashMap<String, PerformanceMetrics>,
    pub(crate) reports: Vec<BrowserStackCollectedReport>,
}

/// BrowserStack transport selected for one Provider Run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowserStackPlatform {
    Espresso,
    XcuiTest,
}

impl BrowserStackPlatform {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Espresso => "espresso",
            Self::XcuiTest => "xcuitest",
        }
    }
}

/// Provider-specific artifacts required to start a BrowserStack run.
#[derive(Clone, Debug)]
pub(crate) enum BrowserStackArtifacts {
    Espresso { app: PathBuf, test_suite: PathBuf },
    XcuiTest { app: PathBuf, test_suite: PathBuf },
}

/// Resolved request accepted by the BrowserStack Adapter.
#[derive(Clone, Debug)]
pub(crate) struct BrowserStackRunRequest {
    pub(crate) devices: Vec<String>,
    pub(crate) artifacts: BrowserStackArtifacts,
}

/// Durable BrowserStack build handle used by delayed collection.
#[derive(Clone, Debug)]
pub(crate) struct BrowserStackRunHandle {
    pub(crate) platform: BrowserStackPlatform,
    pub(crate) requested_devices: Vec<String>,
    pub(crate) app_url: String,
    pub(crate) test_suite_url: Option<String>,
    pub(crate) build_id: String,
}

/// One benchmark report and the telemetry attributed to its Provider Session.
#[derive(Clone, Debug)]
pub(crate) struct BrowserStackReport {
    pub(crate) benchmark: Value,
    pub(crate) performance_metrics: PerformanceMetrics,
    pub(crate) observed_device_id: String,
}

#[derive(Debug, Error)]
#[error(transparent)]
pub(crate) struct BrowserStackAdapterError(#[from] anyhow::Error);

impl BrowserStackAdapterError {
    fn from_anyhow(error: anyhow::Error) -> Self {
        Self(error)
    }
}

#[derive(Debug)]
struct CollectedBrowserStackSession {
    session_id: String,
    benchmark_results: Vec<Value>,
    benchmark_failures: Vec<Value>,
    performance_metrics: PerformanceMetrics,
}

fn browserstack_failure_diagnostic(failures: &[Value]) -> Option<String> {
    let failure = failures.first()?;
    let function = failure
        .get("function_name")
        .and_then(Value::as_str)
        .unwrap_or("unknown function");
    let kind = failure
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown failure");
    let message = failure
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("no message");
    Some(format!(
        "benchmark failure for {function}: {kind}: {message}"
    ))
}

#[cfg(test)]
fn classify_browserstack_result_completeness(
    expected_sessions: &[DeviceSession],
    collected_sessions: Vec<CollectedBrowserStackSession>,
) -> Result<BrowserStackResults> {
    let run = browserstack_adapter_run(expected_sessions, collected_sessions)
        .reconcile()
        .map_err(|error| anyhow!("BrowserStack result set is ambiguous; {error}"))?;
    completed_browserstack_results(run)
}

#[cfg(test)]
fn browserstack_adapter_run(
    expected_sessions: &[DeviceSession],
    collected_sessions: Vec<CollectedBrowserStackSession>,
) -> AdapterRun<BrowserStackReport> {
    let reconciled = expected_sessions
        .iter()
        .cloned()
        .map(|observed| ReconciledDeviceSession {
            requested_device_id: observed.device.clone(),
            observed,
        })
        .collect::<Vec<_>>();
    browserstack_adapter_run_with_bindings(&reconciled, collected_sessions)
}

fn browserstack_adapter_run_with_bindings(
    expected_sessions: &[ReconciledDeviceSession],
    collected_sessions: Vec<CollectedBrowserStackSession>,
) -> AdapterRun<BrowserStackReport> {
    let observed_by_session = expected_sessions
        .iter()
        .map(|session| {
            (
                session.observed.session_id.as_str(),
                session.observed.device.as_str(),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    AdapterRun {
        expected: expected_sessions
            .iter()
            .map(|session| ProviderExpectedSession {
                session_id: session.observed.session_id.clone(),
                device_id: session.requested_device_id.clone(),
                status: session.observed.status.clone(),
            })
            .collect(),
        collected: collected_sessions
            .into_iter()
            .map(|session| {
                let observed_device_id = observed_by_session
                    .get(session.session_id.as_str())
                    .copied()
                    .unwrap_or("unknown")
                    .to_owned();
                CollectedOutput {
                    session_id: session.session_id,
                    reports: session
                        .benchmark_results
                        .into_iter()
                        .map(|benchmark| BrowserStackReport {
                            benchmark,
                            performance_metrics: session.performance_metrics.clone(),
                            observed_device_id: observed_device_id.clone(),
                        })
                        .collect(),
                    failure: browserstack_failure_diagnostic(&session.benchmark_failures),
                }
            })
            .collect(),
    }
}

pub(crate) fn completed_browserstack_results(
    run: ProviderRun<BrowserStackReport>,
) -> Result<BrowserStackResults> {
    let collection = completed_browserstack_collection(run)?;
    Ok((collection.benchmark_results, collection.performance_metrics))
}

pub(crate) fn completed_browserstack_collection(
    run: ProviderRun<BrowserStackReport>,
) -> Result<BrowserStackCollection> {
    if !run.assessment().is_complete() {
        return Err(anyhow!(
            "BrowserStack result set is incomplete; {}",
            run.assessment()
        ));
    }

    let mut benchmark_results = std::collections::HashMap::new();
    let mut performance_metrics = std::collections::HashMap::new();
    let mut reports = Vec::new();
    for (assessment, collected) in run
        .assessment()
        .sessions()
        .iter()
        .zip(run.sessions().iter())
    {
        let report = collected
            .reports
            .first()
            .expect("complete Provider Session has exactly one report");
        reports.push(BrowserStackCollectedReport {
            requested_device_id: assessment.device_id.clone(),
            observed_device_id: report.observed_device_id.clone(),
            transport_session_id: assessment.session_id.clone(),
            benchmark: report.benchmark.clone(),
        });
        benchmark_results.insert(assessment.device_id.clone(), vec![report.benchmark.clone()]);
        if report.performance_metrics.sample_count > 0 {
            performance_metrics.insert(
                assessment.device_id.clone(),
                report.performance_metrics.clone(),
            );
        }
    }

    Ok(BrowserStackCollection {
        benchmark_results,
        performance_metrics,
        reports,
    })
}
use std::time::Instant;

/// Format a file size in human-readable format (MB or KB).
fn format_file_size(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{} MB", bytes / 1_000_000)
    } else if bytes >= 1_000 {
        format!("{} KB", bytes / 1_000)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Get file size from path, returning 0 if unable to read metadata.
fn get_file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// A device available on BrowserStack for testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserStackDevice {
    /// Device name (e.g., "Google Pixel 7", "iPhone 14")
    pub device: String,
    /// Operating system ("android" or "ios")
    pub os: String,
    /// OS version (e.g., "13.0", "16")
    pub os_version: String,
    /// Whether the device is available for testing
    #[serde(default)]
    pub available: Option<bool>,
}

impl BrowserStackDevice {
    /// Returns the device identifier string in BrowserStack format.
    /// Format: "Device Name-OS Version" (e.g., "Google Pixel 7-13.0")
    pub fn identifier(&self) -> String {
        format!("{}-{}", self.device, self.os_version)
    }
}

/// Result of device validation.
#[derive(Debug)]
pub struct DeviceValidationResult {
    /// Valid devices that were matched.
    pub valid: Vec<String>,
    /// Invalid device specs with suggestions.
    pub invalid: Vec<DeviceValidationError>,
}

/// Error details for an invalid device specification.
#[derive(Debug)]
pub struct DeviceValidationError {
    /// The device spec that was provided.
    pub spec: String,
    /// Reason it's invalid.
    pub reason: String,
    /// Suggested alternatives if any match was close.
    pub suggestions: Vec<String>,
}

const DEFAULT_BASE_URL: &str = "https://api-cloud.browserstack.com";
pub(crate) const DEFAULT_BROWSERSTACK_FETCH_TIMEOUT_SECS: u64 = 900;
const ESPRESSO_IDLE_TIMEOUT_SECS: u64 = 900;
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

        let file_size = get_file_size(artifact);
        println!("Uploading Android APK ({})...", format_file_size(file_size));
        let start = Instant::now();

        let form = Form::new().file("file", artifact)?;
        let resp = self
            .http
            .post(self.api("app-automate/espresso/v2/app"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(form)
            .send()
            .context("uploading app to BrowserStack")?;

        let result = parse_response(resp, "app upload")?;
        let elapsed = start.elapsed().as_secs();
        println!("  Uploaded Android APK (took {}s)", elapsed);

        Ok(result)
    }

    /// Upload an Espresso test-suite APK to BrowserStack.
    pub fn upload_espresso_test_suite(&self, artifact: &Path) -> Result<TestSuiteUpload> {
        if !artifact.exists() {
            return Err(anyhow!("test suite artifact not found at {:?}", artifact));
        }

        let file_size = get_file_size(artifact);
        println!(
            "Uploading Android test APK ({})...",
            format_file_size(file_size)
        );
        let start = Instant::now();

        let form = Form::new().file("file", artifact)?;
        let resp = self
            .http
            .post(self.api("app-automate/espresso/v2/test-suite"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(form)
            .send()
            .context("uploading test suite to BrowserStack")?;

        let result = parse_response(resp, "test suite upload")?;
        let elapsed = start.elapsed().as_secs();
        println!("  Uploaded Android test APK (took {}s)", elapsed);

        Ok(result)
    }

    pub fn upload_xcuitest_app(&self, artifact: &Path) -> Result<AppUpload> {
        if !artifact.exists() {
            return Err(anyhow!("iOS app artifact not found at {:?}", artifact));
        }

        let file_size = get_file_size(artifact);
        println!("Uploading iOS app IPA ({})...", format_file_size(file_size));
        let start = Instant::now();

        let form = Form::new().file("file", artifact)?;
        let resp = self
            .http
            .post(self.api("app-automate/xcuitest/v2/app"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(form)
            .send()
            .context("uploading iOS app to BrowserStack")?;

        let result = parse_response(resp, "iOS app upload")?;
        let elapsed = start.elapsed().as_secs();
        println!("  Uploaded iOS app IPA (took {}s)", elapsed);

        Ok(result)
    }

    pub fn upload_xcuitest_test_suite(&self, artifact: &Path) -> Result<TestSuiteUpload> {
        if !artifact.exists() {
            return Err(anyhow!(
                "iOS XCUITest suite artifact not found at {:?}",
                artifact
            ));
        }

        let file_size = get_file_size(artifact);
        println!(
            "Uploading iOS XCUITest runner ({})...",
            format_file_size(file_size)
        );
        let start = Instant::now();

        let form = Form::new().file("file", artifact)?;
        let resp = self
            .http
            .post(self.api("app-automate/xcuitest/v2/test-suite"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(form)
            .send()
            .context("uploading iOS XCUITest suite to BrowserStack")?;

        let result = parse_response(resp, "iOS XCUITest suite upload")?;
        let elapsed = start.elapsed().as_secs();
        println!("  Uploaded iOS XCUITest runner (took {}s)", elapsed);

        Ok(result)
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
            app_profiling: true,
            idle_timeout: ESPRESSO_IDLE_TIMEOUT_SECS,
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
            app_profiling: true,
            build_name: self.project.clone(),
            // Specify the test method to run (required by BrowserStack for XCUITest)
            only_testing: Some(vec![
                "BenchRunnerUITests/BenchRunnerUITests/testLaunchAndCaptureBenchmarkReport"
                    .to_string(),
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
            .asset_request(url)
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

    fn fetch_devices_inventory(&self) -> Result<Vec<BrowserStackDevice>> {
        let json = self.get_json("app-automate/devices.json")?;
        parse_device_list(json, "devices")
    }

    /// List available Android devices for Espresso testing.
    pub fn list_espresso_devices(&self) -> Result<Vec<BrowserStackDevice>> {
        Ok(self
            .fetch_devices_inventory()?
            .into_iter()
            .filter(|device| device.os.eq_ignore_ascii_case("android"))
            .collect())
    }

    /// List available iOS devices for XCUITest testing.
    pub fn list_xcuitest_devices(&self) -> Result<Vec<BrowserStackDevice>> {
        Ok(self
            .fetch_devices_inventory()?
            .into_iter()
            .filter(|device| device.os.eq_ignore_ascii_case("ios"))
            .collect())
    }

    /// List all available devices (both Android and iOS).
    pub fn list_all_devices(&self) -> Result<Vec<BrowserStackDevice>> {
        self.fetch_devices_inventory()
    }

    /// Validate device specifications against available devices.
    ///
    /// Returns a validation result with valid devices and any errors for invalid specs.
    pub fn validate_devices(
        &self,
        specs: &[String],
        platform: Option<&str>,
    ) -> Result<DeviceValidationResult> {
        let available = match platform {
            Some("android") | Some("espresso") => self.list_espresso_devices()?,
            Some("ios") | Some("xcuitest") => self.list_xcuitest_devices()?,
            _ => self.list_all_devices()?,
        };

        let mut valid = Vec::new();
        let mut invalid = Vec::new();

        for spec in specs {
            match validate_device_spec(spec, &available) {
                Ok(matched) => valid.push(matched),
                Err(error) => invalid.push(error),
            }
        }

        Ok(DeviceValidationResult { valid, invalid })
    }

    /// Get the status of an Espresso build
    pub fn get_espresso_build_status(&self, build_id: &str) -> Result<BuildStatus> {
        let path = format!("app-automate/espresso/v2/builds/{}", build_id);
        let json = self.get_json(&path)?;
        build_status_from_value(json).context("parsing build status response")
    }

    /// Get the status of an XCUITest build
    pub fn get_xcuitest_build_status(&self, build_id: &str) -> Result<BuildStatus> {
        let path = format!("app-automate/xcuitest/v2/builds/{}", build_id);
        let json = self.get_json(&path)?;
        build_status_from_value(json).context("parsing build status response")
    }

    /// Fetch device logs for a specific session
    pub fn get_device_logs(
        &self,
        build_id: &str,
        session_id: &str,
        platform: &str,
    ) -> Result<String> {
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

    fn get_session_json(&self, build_id: &str, session_id: &str, platform: &str) -> Result<Value> {
        let path = match platform {
            "espresso" => format!(
                "app-automate/espresso/v2/builds/{}/sessions/{}",
                build_id, session_id
            ),
            "xcuitest" => format!(
                "app-automate/xcuitest/v2/builds/{}/sessions/{}",
                build_id, session_id
            ),
            _ => return Err(anyhow!("unsupported platform: {}", platform)),
        };

        self.get_json(&path)
    }

    fn download_text_url(&self, url: &str) -> Result<String> {
        let resp = self
            .asset_request(url)
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

    /// Extract benchmark results from device logs
    /// Looks for JSON output matching BenchReport format
    /// Supports both Android (BENCH_JSON) and iOS (BENCH_REPORT_JSON_START/END) formats
    pub fn extract_benchmark_results(&self, logs: &str) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        // First, try iOS-style markers: BENCH_REPORT_JSON_START ... BENCH_REPORT_JSON_END
        if let Some(json) = Self::extract_ios_bench_json(logs) {
            Self::extend_unique_results(&mut results, Self::normalize_benchmark_values(json));
        }

        // Decode both the generated chunk protocol and the released legacy frame.
        let android_values = mobench_domain::decode_android_bench_frames(logs)
            .map_err(|error| anyhow!("Malformed Android benchmark framing: {error}"))?;
        for json in android_values {
            Self::extend_unique_results(&mut results, Self::normalize_benchmark_values(json));
        }

        // Look for JSON objects that contain benchmark-related fields (fallback)
        for line in logs.lines() {
            let trimmed = line.trim();
            let looks_like_json = trimmed.starts_with('{') && trimmed.ends_with('}');
            let looks_like_bench =
                trimmed.contains("\"function\"") && trimmed.contains("\"samples\"");
            if (looks_like_json || looks_like_bench)
                && let Ok(json) = serde_json::from_str::<Value>(trimmed)
            {
                Self::extend_unique_results(&mut results, Self::normalize_benchmark_values(json));
            }
        }

        if results.is_empty() {
            Err(anyhow!("No benchmark results found in device logs"))
        } else {
            Ok(results)
        }
    }

    /// Extract structured Android benchmark failures from logs.
    pub fn extract_benchmark_failures(&self, logs: &str) -> Result<Vec<Value>> {
        let mut failures = Vec::new();
        let failure_marker = "BENCH_FAILURE_JSON ";
        for line in logs.lines() {
            if let Some(idx) = line.find(failure_marker) {
                let json_part = &line[idx + failure_marker.len()..];
                if let Ok(json) = serde_json::from_str::<Value>(json_part) {
                    Self::extend_unique_results(&mut failures, vec![json]);
                }
            }
        }

        if failures.is_empty() {
            Err(anyhow!("No benchmark failures found in device logs"))
        } else {
            Ok(failures)
        }
    }

    pub(crate) fn extract_benchmark_results_from_artifact(
        &self,
        contents: &str,
    ) -> Result<Vec<Value>> {
        let trimmed = contents.trim();
        if !trimmed.is_empty()
            && let Ok(json) = serde_json::from_str::<Value>(trimmed)
        {
            let results = Self::normalize_benchmark_values(json);
            if !results.is_empty() {
                return Ok(results);
            }
        }

        self.extract_benchmark_results(contents)
    }

    pub(crate) fn extract_results_from_session_artifacts<F>(
        &self,
        session_json: &Value,
        mut fetch_text: F,
    ) -> Result<(Vec<Value>, PerformanceMetrics)>
    where
        F: FnMut(&str) -> Result<String>,
    {
        let artifact_urls = Self::collect_text_artifact_urls(session_json);
        if artifact_urls.is_empty() {
            return Err(anyhow!("No text artifact URLs found in session response"));
        }

        let mut benchmark_results = Vec::new();
        let mut snapshots = Vec::new();

        for (_, url) in artifact_urls {
            let contents = match fetch_text(&url) {
                Ok(contents) => contents,
                Err(_) => continue,
            };

            if benchmark_results.is_empty()
                && let Ok(results) = self.extract_benchmark_results_from_artifact(&contents)
            {
                benchmark_results = results;
            }

            if let Ok(mut artifact_snapshots) = self.extract_performance_snapshots(&contents) {
                snapshots.append(&mut artifact_snapshots);
            }
        }

        if benchmark_results.is_empty() {
            Err(anyhow!("No benchmark results found in session artifacts"))
        } else {
            Ok((
                benchmark_results,
                PerformanceMetrics::from_snapshots(snapshots),
            ))
        }
    }

    pub(crate) fn extract_failures_from_session_artifacts<F>(
        &self,
        session_json: &Value,
        mut fetch_text: F,
    ) -> Result<Vec<Value>>
    where
        F: FnMut(&str) -> Result<String>,
    {
        let artifact_urls = Self::collect_text_artifact_urls(session_json);
        if artifact_urls.is_empty() {
            return Err(anyhow!("No text artifact URLs found in session response"));
        }

        let mut failures = Vec::new();
        for (_, url) in artifact_urls {
            let Ok(contents) = fetch_text(&url) else {
                continue;
            };
            if let Ok(mut artifact_failures) = self.extract_benchmark_failures(&contents) {
                failures.append(&mut artifact_failures);
            }
        }

        if failures.is_empty() {
            Err(anyhow!("No benchmark failures found in session artifacts"))
        } else {
            Ok(failures)
        }
    }

    /// Extract benchmark JSON from iOS logs using START/END markers.
    /// iOS uses NSLog which may split the JSON across multiple log lines.
    fn extract_ios_bench_json(logs: &str) -> Option<Value> {
        let start_marker = "BENCH_REPORT_JSON_START";
        let end_marker = "BENCH_REPORT_JSON_END";

        // Find the last occurrence of start marker (in case of multiple runs)
        let start_pos = logs.rfind(start_marker)?;
        let after_start = &logs[start_pos + start_marker.len()..];

        // Find the end marker after the start
        let end_pos = after_start.find(end_marker)?;
        let json_section = &after_start[..end_pos];

        // Try to extract valid JSON from the section
        Self::extract_json_from_ios_log_section(json_section)
    }

    /// Extract valid JSON from an iOS log section that may contain log prefixes/timestamps.
    fn extract_json_from_ios_log_section(section: &str) -> Option<Value> {
        // First, try the whole section as-is (trimmed)
        let trimmed = section.trim();
        if trimmed.starts_with('{')
            && trimmed.ends_with('}')
            && let Ok(json) = serde_json::from_str::<Value>(trimmed)
        {
            return Some(json);
        }

        // Look for JSON on individual lines, stripping iOS log prefixes
        for line in section.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Look for JSON starting with {
            if let Some(json_start) = line.find('{') {
                let potential_json = &line[json_start..];
                if let Some(json) = Self::extract_balanced_json(potential_json)
                    && let Ok(parsed) = serde_json::from_str::<Value>(&json)
                {
                    return Some(parsed);
                }
            }
        }

        // Try concatenating all lines (for multi-line JSON)
        let all_content: String = section
            .lines()
            .map(|line| {
                // Strip common iOS log prefixes (timestamps, process info)
                // Format: "2026-01-20 12:34:56.789 AppName[pid:tid] content"
                if let Some(bracket_end) = line.find("] ") {
                    &line[bracket_end + 2..]
                } else {
                    line.trim()
                }
            })
            .collect::<Vec<_>>()
            .join("");

        if let Some(json_start) = all_content.find('{') {
            let potential_json = &all_content[json_start..];
            if let Some(json) = Self::extract_balanced_json(potential_json)
                && let Ok(parsed) = serde_json::from_str::<Value>(&json)
            {
                return Some(parsed);
            }
        }

        None
    }

    /// Extract a balanced JSON object from a string starting with '{'.
    fn extract_balanced_json(s: &str) -> Option<String> {
        if !s.starts_with('{') {
            return None;
        }

        let mut depth = 0;
        let mut in_string = false;
        let mut escape_next = false;

        for (i, c) in s.char_indices() {
            if escape_next {
                escape_next = false;
                continue;
            }

            match c {
                '\\' if in_string => {
                    escape_next = true;
                }
                '"' => {
                    in_string = !in_string;
                }
                '{' if !in_string => {
                    depth += 1;
                }
                '}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(s[..=i].to_string());
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Extract performance metrics from device logs
    /// Looks for JSON objects with "type":"performance" or similar performance indicators
    pub fn extract_performance_metrics(&self, logs: &str) -> Result<PerformanceMetrics> {
        Ok(PerformanceMetrics::from_snapshots(
            self.extract_performance_snapshots(logs)?,
        ))
    }

    fn extract_performance_snapshots(&self, logs: &str) -> Result<Vec<PerformanceSnapshot>> {
        let mut snapshots = Vec::new();

        for line in logs.lines() {
            let trimmed = line.trim();
            let looks_like_json = trimmed.starts_with('{') && trimmed.ends_with('}');
            if looks_like_json
                && let Ok(json) = serde_json::from_str::<Value>(trimmed)
                && (json.get("type").and_then(|t| t.as_str()) == Some("performance")
                    || json.get("memory").is_some()
                    || json.get("cpu").is_some())
                && let Ok(snapshot) = serde_json::from_value::<PerformanceSnapshot>(json)
            {
                snapshots.push(snapshot);
            }
        }

        Ok(snapshots)
    }

    /// Wait for build completion and fetch all results including performance metrics.
    ///
    /// Convenience wrapper around [`Self::wait_and_fetch_all_results_with_poll`]
    /// with default poll interval.
    #[allow(dead_code)]
    pub fn wait_and_fetch_all_results(
        &self,
        build_id: &str,
        platform: &str,
        timeout_secs: Option<u64>,
    ) -> Result<BrowserStackResults> {
        self.wait_and_fetch_all_results_with_poll(build_id, platform, timeout_secs, None)
    }

    pub fn wait_and_fetch_all_results_with_poll(
        &self,
        build_id: &str,
        platform: &str,
        timeout_secs: Option<u64>,
        poll_interval_secs: Option<u64>,
    ) -> Result<BrowserStackResults> {
        let platform = match platform {
            "espresso" => BrowserStackPlatform::Espresso,
            "xcuitest" => BrowserStackPlatform::XcuiTest,
            _ => return Err(anyhow!("unsupported platform: {platform}")),
        };
        let run = self
            .wait_and_collect_adapter_run(
                build_id,
                platform,
                None,
                timeout_secs.unwrap_or(DEFAULT_BROWSERSTACK_FETCH_TIMEOUT_SECS),
                poll_interval_secs.unwrap_or(5),
                &mobench_process::global_cancellation_token(),
            )?
            .reconcile()
            .map_err(|error| anyhow!("BrowserStack result set is ambiguous; {error}"))?;
        completed_browserstack_results(run)
    }

    fn wait_and_collect_adapter_run(
        &self,
        build_id: &str,
        platform: BrowserStackPlatform,
        requested_devices: Option<&[String]>,
        timeout: u64,
        poll_interval: u64,
        cancellation: &ProcessCancellation,
    ) -> Result<AdapterRun<BrowserStackReport>> {
        let platform = platform.as_str();

        println!(
            "Waiting for build {} to complete (timeout: {}s, poll: {}s)...",
            build_id, timeout, poll_interval
        );
        let build_status = self.poll_build_completion_cancellable(
            build_id,
            platform,
            timeout,
            poll_interval,
            true,
            cancellation,
        )?;
        let verified_sessions = match requested_devices {
            Some(requested_devices) => {
                reconcile_requested_device_sessions(requested_devices, &build_status.devices)?
            }
            None => build_status
                .devices
                .iter()
                .cloned()
                .map(|observed| ReconciledDeviceSession {
                    requested_device_id: observed.device.clone(),
                    observed,
                })
                .collect(),
        };

        println!("Build completed with status: {}", build_status.status);
        println!(
            "Fetching results from {} device(s)...",
            verified_sessions.len()
        );

        let mut collected_sessions = Vec::with_capacity(verified_sessions.len());

        for reconciled in &verified_sessions {
            let device = &reconciled.observed;
            if cancellation.is_cancelled() {
                return Err(anyhow!("BrowserStack collection was cancelled"));
            }
            println!(
                "  Fetching logs for {} (session: {})...",
                device.device, device.session_id
            );

            let mut device_benchmark_results: Option<Vec<Value>> = None;
            let mut device_performance_metrics = PerformanceMetrics::default();
            let mut device_failures: Option<Vec<Value>> = None;

            match self.get_device_logs(build_id, &device.session_id, platform) {
                Ok(logs) => {
                    // Extract benchmark results
                    match self.extract_benchmark_results(&logs) {
                        Ok(results) => {
                            println!("    Found {} benchmark result(s)", results.len());
                            device_benchmark_results = Some(results);
                        }
                        Err(e) => {
                            println!("    No benchmark results in live logs: {}", e);
                        }
                    }

                    match self.extract_benchmark_failures(&logs) {
                        Ok(failures) => {
                            println!("    Found {} benchmark failure marker(s)", failures.len());
                            device_failures = Some(failures);
                        }
                        Err(e) => {
                            println!("    No benchmark failures in live logs: {}", e);
                        }
                    }

                    // Extract performance metrics
                    match self.extract_performance_metrics(&logs) {
                        Ok(perf_metrics) if perf_metrics.sample_count > 0 => {
                            println!(
                                "    Found {} performance metric snapshot(s)",
                                perf_metrics.sample_count
                            );
                            device_performance_metrics = perf_metrics;
                        }
                        Ok(_) => {
                            println!("    No performance metrics found in live logs");
                        }
                        Err(e) => {
                            println!("    Warning: Failed to extract performance metrics - {}", e);
                        }
                    }
                }
                Err(e) => {
                    println!("    Failed to fetch live logs: {}", e);
                }
            }

            let should_fetch_failure_artifacts = device_failures.is_none()
                && (device_benchmark_results.is_none()
                    || !device.status.eq_ignore_ascii_case("passed"));
            if device_benchmark_results.is_none() || should_fetch_failure_artifacts {
                match self.get_session_json(build_id, &device.session_id, platform) {
                    Ok(session_json) => {
                        if device_benchmark_results.is_none() {
                            match self
                                .extract_results_from_session_artifacts(&session_json, |url| {
                                    self.download_text_url(url)
                                }) {
                                Ok((results, perf_metrics)) => {
                                    println!(
                                        "    Found {} benchmark result(s) from session artifacts",
                                        results.len()
                                    );
                                    if device_performance_metrics.sample_count == 0
                                        && perf_metrics.sample_count > 0
                                    {
                                        println!(
                                            "    Found {} performance metric snapshot(s) from session artifacts",
                                            perf_metrics.sample_count
                                        );
                                        device_performance_metrics = perf_metrics;
                                    }
                                    device_benchmark_results = Some(results);
                                }
                                Err(e) => {
                                    println!(
                                        "    Warning: Failed to fetch results from session artifacts: {e}"
                                    );
                                }
                            }
                        }

                        if should_fetch_failure_artifacts
                            && let Ok(failures) = self
                                .extract_failures_from_session_artifacts(&session_json, |url| {
                                    self.download_text_url(url)
                                })
                        {
                            println!(
                                "    Found {} benchmark failure marker(s) from session artifacts",
                                failures.len()
                            );
                            device_failures = Some(failures);
                        }
                    }
                    Err(e) => {
                        println!("    Warning: Failed to fetch session artifacts metadata: {e}");
                    }
                }
            }

            if let Ok(app_profiling_v2) = self.get_app_profiling_v2(build_id, &device.session_id)
                && app_profiling_v2.sample_count > 0
            {
                println!("    Found App Profiling v2 metrics");
                device_performance_metrics = merge_performance_metrics(
                    Some(device_performance_metrics),
                    Some(app_profiling_v2),
                )
                .unwrap_or_default();
            }

            collected_sessions.push(CollectedBrowserStackSession {
                session_id: device.session_id.clone(),
                benchmark_results: device_benchmark_results.unwrap_or_default(),
                benchmark_failures: device_failures.unwrap_or_default(),
                performance_metrics: device_performance_metrics,
            });
        }

        Ok(browserstack_adapter_run_with_bindings(
            &verified_sessions,
            collected_sessions,
        ))
    }

    /// Fetch session details from BrowserStack API.
    pub fn get_session_details(&self, build_id: &str, session_id: &str) -> Result<SessionDetails> {
        let path = format!("/app-automate/builds/{build_id}/sessions/{session_id}");
        let value = self.get_json(&path)?;

        let automation_session = value
            .get("automation_session")
            .context("Missing automation_session in response")?;

        Ok(SessionDetails {
            device: automation_session
                .get("device")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            os: automation_session
                .get("os")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            os_version: automation_session
                .get("os_version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            duration: automation_session.get("duration").and_then(|v| v.as_u64()),
        })
    }

    /// Fetch App Profiling v2 metrics for a BrowserStack session.
    pub fn get_app_profiling_v2(
        &self,
        build_id: &str,
        session_id: &str,
    ) -> Result<PerformanceMetrics> {
        let path = format!("/app-automate/builds/{build_id}/sessions/{session_id}/appprofiling/v2");
        let value = self.get_json(&path)?;
        parse_app_profiling_v2_response(&value)
            .with_context(|| format!("parsing App Profiling v2 for session {session_id}"))
    }

    /// Fetch build details with all sessions and performance data.
    pub fn get_build_summary(&self, build_id: &str, platform: &str) -> Result<BuildSummary> {
        let status = match platform {
            "ios" => self.get_xcuitest_build_status(build_id)?,
            _ => self.get_espresso_build_status(build_id)?,
        };

        let mut sessions = Vec::new();
        for device_session in &status.devices {
            let details = self
                .get_session_details(build_id, &device_session.session_id)
                .ok();

            let perf = device_session
                .device_logs
                .as_ref()
                .and_then(|logs| self.extract_performance_metrics(logs).ok());
            let app_profiling_v2 = self
                .get_app_profiling_v2(build_id, &device_session.session_id)
                .ok();

            sessions.push(SessionSummary {
                session_id: device_session.session_id.clone(),
                device: details
                    .as_ref()
                    .map(|d| d.device.clone())
                    .unwrap_or_else(|| device_session.device.clone()),
                os: details.as_ref().map(|d| d.os.clone()).unwrap_or_default(),
                os_version: details
                    .as_ref()
                    .map(|d| d.os_version.clone())
                    .unwrap_or_default(),
                duration_secs: details.as_ref().and_then(|d| d.duration),
                performance: merge_performance_metrics(perf, app_profiling_v2),
            });
        }

        Ok(BuildSummary {
            build_id: build_id.to_string(),
            status: status.status,
            sessions,
        })
    }

    fn normalize_benchmark_values(value: Value) -> Vec<Value> {
        match value {
            Value::Array(entries) => entries
                .into_iter()
                .filter_map(Self::normalize_benchmark_value)
                .collect(),
            value => Self::normalize_benchmark_value(value).into_iter().collect(),
        }
    }

    fn normalize_benchmark_value(mut value: Value) -> Option<Value> {
        let samples = Self::extract_sample_durations(&value);
        let stats = Distribution::from_slice(&samples).cli_v1_summary();
        let object = value.as_object_mut()?;

        if !object.contains_key("function")
            && let Some(function) = object
                .get("spec")
                .and_then(|spec| spec.get("name"))
                .and_then(|name| name.as_str())
        {
            object.insert("function".to_string(), Value::String(function.to_string()));
        }

        if !object.contains_key("samples")
            && let Some(samples_ns) = object
                .get("samples_ns")
                .and_then(|samples| samples.as_array())
        {
            object.insert("samples".to_string(), Value::Array(samples_ns.clone()));
        }

        let has_function = object
            .get("function")
            .and_then(|value| value.as_str())
            .is_some();
        let has_samples = object
            .get("samples")
            .and_then(|value| value.as_array())
            .is_some();
        let has_stats = ["mean_ns", "median_ns", "p95_ns", "min_ns", "max_ns"]
            .iter()
            .any(|key| object.get(*key).is_some());

        if !has_function || (!has_samples && !has_stats) {
            return None;
        }

        if let Some(stats) = stats {
            if !object.contains_key("mean_ns") {
                object.insert("mean_ns".to_string(), Value::from(stats.mean_ns));
            }
            if !object.contains_key("median_ns") {
                object.insert("median_ns".to_string(), Value::from(stats.median_ns));
            }
            if !object.contains_key("p95_ns") {
                object.insert("p95_ns".to_string(), Value::from(stats.p95_ns));
            }
            if !object.contains_key("min_ns") {
                object.insert("min_ns".to_string(), Value::from(stats.min_ns));
            }
            if !object.contains_key("max_ns") {
                object.insert("max_ns".to_string(), Value::from(stats.max_ns));
            }
        }

        Some(value)
    }

    fn extend_unique_results(results: &mut Vec<Value>, mut new_results: Vec<Value>) {
        for result in new_results.drain(..) {
            if !results.iter().any(|existing| existing == &result) {
                results.push(result);
            }
        }
    }

    fn collect_text_artifact_urls(value: &Value) -> Vec<(String, String)> {
        let mut urls = Vec::new();
        Self::collect_text_artifact_urls_recursive(value, "", &mut urls);
        urls.sort_by_key(|(key, url)| Self::artifact_url_priority(key, url));
        urls
    }

    fn collect_text_artifact_urls_recursive(
        value: &Value,
        prefix: &str,
        out: &mut Vec<(String, String)>,
    ) {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    let next = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", prefix, key)
                    };
                    if let Value::String(url) = value
                        && (url.starts_with("http") || url.starts_with("bs://"))
                        && Self::artifact_url_priority(&next, url) < 4
                    {
                        out.push((next.clone(), url.clone()));
                    }
                    Self::collect_text_artifact_urls_recursive(value, &next, out);
                }
            }
            Value::Array(items) => {
                for (index, value) in items.iter().enumerate() {
                    let next = format!("{}[{}]", prefix, index);
                    Self::collect_text_artifact_urls_recursive(value, &next, out);
                }
            }
            _ => {}
        }
    }

    fn artifact_url_priority(key: &str, url: &str) -> u8 {
        let lower = format!("{} {}", key.to_ascii_lowercase(), url.to_ascii_lowercase());
        if lower.contains("bench-report") || lower.contains("bench_report") {
            0
        } else if lower.contains("device_log")
            || lower.contains("devicelog")
            || lower.contains("instrumentation_log")
            || lower.contains("app_log")
        {
            1
        } else if lower.ends_with(".json") || lower.ends_with(".log") || lower.ends_with(".txt") {
            2
        } else {
            4
        }
    }

    fn extract_sample_durations(value: &Value) -> Vec<u64> {
        let mut durations = Vec::new();

        if let Some(samples) = value.get("samples").and_then(|samples| samples.as_array()) {
            for sample in samples {
                if let Some(duration_ns) = sample
                    .get("duration_ns")
                    .and_then(|duration| duration.as_u64())
                {
                    durations.push(duration_ns);
                } else if let Some(duration_ns) = sample.as_u64() {
                    durations.push(duration_ns);
                }
            }
        }

        if durations.is_empty()
            && let Some(samples_ns) = value
                .get("samples_ns")
                .and_then(|samples| samples.as_array())
        {
            durations.extend(samples_ns.iter().filter_map(|sample| sample.as_u64()));
        }

        durations
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
                peak_mb: memory_values
                    .iter()
                    .fold(f64::NEG_INFINITY, |a, &b| a.max(b)),
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

fn build_status_from_value(value: Value) -> Result<BuildStatus> {
    if let Ok(response) = serde_json::from_value::<BuildStatusResponse>(value.clone()) {
        return Ok(response.into());
    }

    let build_id = value
        .get("build_id")
        .or_else(|| value.get("buildId"))
        .or_else(|| value.get("id"))
        .and_then(|val| val.as_str())
        .ok_or_else(|| anyhow!("build status response missing build id"))?
        .to_string();
    let status = value
        .get("status")
        .and_then(|val| val.as_str())
        .unwrap_or("unknown")
        .to_string();
    let duration = value.get("duration").and_then(|val| val.as_u64());

    let mut devices = Vec::new();
    if let Some(entries) = value.get("devices").and_then(|val| val.as_array()) {
        for entry in entries {
            let device_name = entry
                .get("device")
                .and_then(|val| val.as_str())
                .context("BrowserStack build device entry missing device name")?;
            let observed_device_id = if is_valid_device_selector(device_name) {
                device_name.to_owned()
            } else {
                let os_version = entry
                    .get("os_version")
                    .or_else(|| entry.get("osVersion"))
                    .and_then(Value::as_str)
                    .context("BrowserStack build device entry missing OS version")?;
                BrowserStackDevice {
                    device: device_name.to_owned(),
                    os: entry
                        .get("os")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned(),
                    os_version: os_version.to_owned(),
                    available: None,
                }
                .identifier()
            };
            if let Some(sessions) = entry.get("sessions").and_then(|val| val.as_array()) {
                for session in sessions {
                    let session_id = session
                        .get("id")
                        .or_else(|| session.get("session_id"))
                        .or_else(|| session.get("sessionId"))
                        .and_then(|val| val.as_str());
                    if let Some(session_id) = session_id {
                        let session_status = session
                            .get("status")
                            .and_then(|val| val.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        devices.push(DeviceSession {
                            device: observed_device_id.clone(),
                            session_id: session_id.to_string(),
                            status: session_status,
                            device_logs: None,
                        });
                    }
                }
            }
        }
    }

    Ok(BuildStatus {
        build_id,
        status,
        duration,
        devices,
    })
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

fn merge_performance_metrics(
    base: Option<PerformanceMetrics>,
    preferred: Option<PerformanceMetrics>,
) -> Option<PerformanceMetrics> {
    match (base, preferred) {
        (None, None) => None,
        (Some(base), None) => Some(base),
        (None, Some(preferred)) => Some(preferred),
        (Some(mut base), Some(preferred)) => {
            if preferred.memory.is_some() {
                base.memory = preferred.memory;
            }
            if preferred.cpu.is_some() {
                base.cpu = preferred.cpu;
            }
            if !preferred.snapshots.is_empty() {
                base.snapshots = preferred.snapshots;
            }
            base.sample_count = base.sample_count.max(preferred.sample_count);
            Some(base)
        }
    }
}

fn parse_app_profiling_v2_response(value: &Value) -> Result<PerformanceMetrics> {
    let data = value
        .get("data")
        .and_then(|data| data.as_object())
        .context("App Profiling v2 response missing data object")?;

    let mut selected_metrics = None;

    for (app_id, app_data) in data {
        if app_id == "units" {
            continue;
        }

        let status = app_data.get("status").and_then(|status| status.as_str());
        let metrics = app_data.get("metrics");
        if status == Some("success") && metrics.is_some() {
            selected_metrics = metrics;
            break;
        }

        if selected_metrics.is_none() && metrics.is_some() {
            selected_metrics = metrics;
        }
    }

    let metrics = selected_metrics
        .and_then(|metrics| metrics.as_object())
        .context("App Profiling v2 response missing metrics payload")?;

    let cpu_avg = metrics
        .get("cpu")
        .and_then(|cpu| cpu.get("avg"))
        .and_then(|value| value.as_f64());
    let cpu_max = metrics
        .get("cpu")
        .and_then(|cpu| cpu.get("max"))
        .and_then(|value| value.as_f64());
    let mem_avg = metrics
        .get("mem")
        .and_then(|mem| mem.get("avg"))
        .and_then(|value| value.as_f64());
    let mem_max = metrics
        .get("mem")
        .and_then(|mem| mem.get("max"))
        .and_then(|value| value.as_f64());

    let cpu = match (cpu_avg, cpu_max) {
        (None, None) => None,
        (avg, max) => {
            let average_percent = avg.or(max).unwrap_or_default();
            let peak_percent = max.or(avg).unwrap_or_default();
            Some(AggregateCpuMetrics {
                peak_percent,
                average_percent,
                min_percent: average_percent.min(peak_percent),
            })
        }
    };

    let memory = match (mem_avg, mem_max) {
        (None, None) => None,
        (avg, max) => {
            let average_mb = avg.or(max).unwrap_or_default();
            let peak_mb = max.or(avg).unwrap_or_default();
            Some(AggregateMemoryMetrics {
                peak_mb,
                average_mb,
                min_mb: average_mb.min(peak_mb),
            })
        }
    };

    let sample_count = usize::from(cpu.is_some() || memory.is_some());

    Ok(PerformanceMetrics {
        sample_count,
        memory,
        cpu,
        snapshots: Vec::new(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildRequest {
    app: String,
    test_suite: String,
    devices: Vec<String>,
    device_logs: bool,
    disable_animations: bool,
    app_profiling: bool,
    idle_timeout: u64,
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
    app_profiling: bool,
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

fn should_authenticate_asset_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };

    host == "browserstack.com" || host.ends_with(".browserstack.com")
}

/// Parse a device list response from BrowserStack API.
fn parse_device_list(json: Value, context: &str) -> Result<Vec<BrowserStackDevice>> {
    // BrowserStack returns an array of device objects
    let devices = match json {
        Value::Array(arr) => arr,
        Value::Object(obj) => {
            // Some endpoints wrap the list in a "devices" key
            obj.get("devices")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        }
        _ => {
            return Err(anyhow!(
                "Unexpected response format from {} devices endpoint",
                context
            ));
        }
    };

    let mut result = Vec::with_capacity(devices.len());
    for device in devices {
        // Handle both flat format and nested format
        let device_name = device
            .get("device")
            .or_else(|| device.get("name"))
            .or_else(|| device.get("deviceName"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let os = device
            .get("os")
            .and_then(|v| v.as_str())
            .unwrap_or(if context == "xcuitest" {
                "ios"
            } else {
                "android"
            })
            .to_string();

        let os_version = device
            .get("os_version")
            .or_else(|| device.get("osVersion"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let available = device
            .get("available")
            .or_else(|| device.get("realMobile"))
            .and_then(|v| v.as_bool());

        result.push(BrowserStackDevice {
            device: device_name,
            os,
            os_version,
            available,
        });
    }

    Ok(result)
}

/// Validate a device specification against available devices.
///
/// The spec can be:
/// - Exact match: "Google Pixel 7-13.0"
/// - Device name only: "Google Pixel 7" (matches any version)
/// - Partial match: "Pixel 7" (fuzzy match)
///
/// Provides improved suggestions:
/// - If user types "Pixel 7", suggests "Google Pixel 7-13.0", "Google Pixel 7-14.0"
/// - If OS version doesn't match, suggests same device with available versions
/// - Shows top 3 suggestions max
fn validate_device_spec(
    spec: &str,
    available: &[BrowserStackDevice],
) -> std::result::Result<String, DeviceValidationError> {
    let spec_lower = spec.to_lowercase();

    // First, try exact match on identifier
    for device in available {
        if device.identifier().to_lowercase() == spec_lower {
            return Ok(device.identifier());
        }
    }

    // Try matching device name only (for specs without version)
    if !spec.contains('-') {
        for device in available {
            if device.device.to_lowercase() == spec_lower {
                // Return the full identifier with version
                return Ok(device.identifier());
            }
        }
    }

    // Parse spec to see if it has a version component
    let (spec_device, spec_version) = if let Some(dash_pos) = spec.rfind('-') {
        let device_part = &spec[..dash_pos];
        let version_part = &spec[dash_pos + 1..];
        // Only treat as version if it looks like a version number
        if version_part
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            (
                device_part.to_lowercase(),
                Some(version_part.to_lowercase()),
            )
        } else {
            (spec_lower.clone(), None)
        }
    } else {
        (spec_lower.clone(), None)
    };

    // Check if the device name matches but OS version is wrong
    if let Some(ref version) = spec_version {
        let matching_devices: Vec<&BrowserStackDevice> = available
            .iter()
            .filter(|d| d.device.to_lowercase() == spec_device)
            .collect();

        if !matching_devices.is_empty() {
            // Device exists but with different versions
            let available_versions: Vec<String> =
                matching_devices.iter().map(|d| d.identifier()).collect();

            let mut suggestions = available_versions;
            suggestions.sort();
            suggestions.truncate(3);

            return Err(DeviceValidationError {
                spec: spec.to_string(),
                reason: format!("OS version '{}' not available for this device", version),
                suggestions,
            });
        }
    }

    // Try fuzzy matching - prioritize matches that start with the spec
    let mut scored_suggestions: Vec<(u32, String)> = Vec::new();
    for device in available {
        let id = device.identifier();
        let id_lower = id.to_lowercase();
        let device_lower = device.device.to_lowercase();

        // Score based on how well the spec matches
        let score = if device_lower.starts_with(&spec_device) {
            // High priority: device name starts with spec
            100
        } else if device_lower.contains(&spec_device) {
            // Medium priority: device name contains spec
            50
        } else if id_lower.contains(&spec_lower) {
            // Lower priority: full identifier contains spec
            25
        } else {
            // Check for partial word matches (e.g., "Pixel 7" in "Google Pixel 7")
            let spec_words: Vec<&str> = spec_lower.split_whitespace().collect();
            let device_words: Vec<&str> = device_lower.split_whitespace().collect();

            let matches = spec_words
                .iter()
                .filter(|sw| device_words.iter().any(|dw| dw.contains(*sw)))
                .count();

            if matches == spec_words.len() && !spec_words.is_empty() {
                // All words from spec found in device name
                75
            } else if matches > 0 {
                // Some words match
                10 * matches as u32
            } else {
                0
            }
        };

        if score > 0 {
            scored_suggestions.push((score, id));
        }
    }

    // Sort by score (descending), then alphabetically
    scored_suggestions.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    // Take top 3 unique suggestions
    let suggestions: Vec<String> = scored_suggestions
        .into_iter()
        .map(|(_, id)| id)
        .take(3)
        .collect();

    Err(DeviceValidationError {
        spec: spec.to_string(),
        reason: if suggestions.is_empty() {
            "No matching device found".to_string()
        } else {
            "Device not found, but similar devices are available".to_string()
        },
        suggestions,
    })
}

/// Details about a single BrowserStack session (from the session API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetails {
    pub device: String,
    pub os: String,
    pub os_version: String,
    pub duration: Option<u64>,
}

/// High-level summary of a BrowserStack build with all sessions and their metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSummary {
    pub build_id: String,
    pub status: String,
    pub sessions: Vec<SessionSummary>,
}

/// Summary of a single session within a build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub device: String,
    pub os: String,
    pub os_version: String,
    pub duration_secs: Option<u64>,
    pub performance: Option<PerformanceMetrics>,
}

/// Format a helpful error message for missing BrowserStack credentials.
pub fn format_credentials_error(_missing_username: bool, _missing_access_key: bool) -> String {
    let mut message = String::from("BrowserStack credentials not configured.\n\n");

    message.push_str("Set credentials using one of these methods:\n\n");

    message.push_str("  1. Environment variables:\n");
    message.push_str("     export BROWSERSTACK_USERNAME=your_username\n");
    message.push_str("     export BROWSERSTACK_ACCESS_KEY=your_access_key\n\n");

    message.push_str("  2. Config file (bench-config.toml):\n");
    message.push_str("     [browserstack]\n");
    message.push_str("     app_automate_username = \"your_username\"\n");
    message.push_str("     app_automate_access_key = \"your_access_key\"\n\n");

    message.push_str("  3. .env.local file in project root:\n");
    message.push_str("     BROWSERSTACK_USERNAME=your_username\n");
    message.push_str("     BROWSERSTACK_ACCESS_KEY=your_access_key\n\n");

    message.push_str("Get credentials: https://app-automate.browserstack.com/\n");
    message.push_str("(Navigate to Settings -> Access Key)\n");

    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    fn test_client_with_base_url(base_url: impl Into<String>) -> BrowserStackClient {
        BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap()
        .with_base_url(base_url)
    }

    fn spawn_browserstack_json_server(
        payload: Value,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        listener
            .set_nonblocking(true)
            .expect("configure nonblocking listener");
        let addr = listener.local_addr().expect("read test server address");
        let paths = Arc::new(Mutex::new(Vec::new()));
        let recorded_paths = Arc::clone(&paths);
        let body = payload.to_string();

        let handle = thread::spawn(move || {
            let mut last_activity = Instant::now();
            loop {
                match listener.accept() {
                    Ok((mut stream, _peer)) => {
                        last_activity = Instant::now();
                        stream.set_nonblocking(false).expect("set stream blocking");

                        let mut buf = [0_u8; 4096];
                        let bytes_read = stream.read(&mut buf).expect("read request");
                        let request = String::from_utf8_lossy(&buf[..bytes_read]);
                        let path = request
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().nth(1))
                            .unwrap_or("/")
                            .to_string();

                        recorded_paths.lock().unwrap().push(path.clone());

                        let (status, response_body) = if path == "/app-automate/devices.json" {
                            ("200 OK", body.clone())
                        } else {
                            (
                                "404 Not Found",
                                format!("{{\"error\":\"unexpected path: {path}\"}}"),
                            )
                        };

                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            response_body.len(),
                            response_body
                        );
                        stream
                            .write_all(response.as_bytes())
                            .expect("write response");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if last_activity.elapsed() >= Duration::from_secs(2) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept request: {error}"),
                }
            }
        });

        (format!("http://{addr}"), paths, handle)
    }

    fn spawn_browserstack_path_json_server(
        expected_path: &'static str,
        payload: Value,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("read test server address");
        let body = payload.to_string();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buf = [0_u8; 4096];
            let bytes_read = stream.read(&mut buf).expect("read request");
            let request = String::from_utf8_lossy(&buf[..bytes_read]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let (status, response_body) = if path == expected_path {
                ("200 OK", body)
            } else {
                (
                    "404 Not Found",
                    format!("{{\"error\":\"unexpected path: {path}\"}}"),
                )
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });

        (format!("http://{addr}"), handle)
    }

    struct ScriptedHttpResponse {
        path: &'static str,
        status: &'static str,
        content_type: &'static str,
        body: String,
    }

    impl ScriptedHttpResponse {
        fn json(path: &'static str, body: Value) -> Self {
            Self {
                path,
                status: "200 OK",
                content_type: "application/json",
                body: body.to_string(),
            }
        }

        fn text(path: &'static str, body: impl Into<String>) -> Self {
            Self {
                path,
                status: "200 OK",
                content_type: "text/plain",
                body: body.into(),
            }
        }

        fn not_found(path: &'static str) -> Self {
            Self {
                path,
                status: "404 Not Found",
                content_type: "application/json",
                body: json!({"error": "not found"}).to_string(),
            }
        }
    }

    fn read_scripted_request(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set request read timeout");
        let mut request = Vec::new();
        let mut header_end = None;
        let mut content_length = 0usize;
        loop {
            let mut buffer = [0_u8; 4096];
            let bytes_read = stream.read(&mut buffer).expect("read scripted request");
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..bytes_read]);
            if header_end.is_none()
                && let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n")
            {
                let end = index + 4;
                let headers = String::from_utf8_lossy(&request[..end]);
                content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or_default();
                header_end = Some(end);
            }
            if header_end.is_some_and(|end| request.len() >= end + content_length) {
                break;
            }
        }
        String::from_utf8_lossy(&request).into_owned()
    }

    fn spawn_browserstack_script_server(
        responses: Vec<ScriptedHttpResponse>,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind scripted test server");
        let addr = listener.local_addr().expect("read scripted server address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded_requests = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept scripted request");
                let request = read_scripted_request(&mut stream);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                assert_eq!(path, response.path, "unexpected scripted request");
                recorded_requests.lock().unwrap().push(request);
                let wire_response = format!(
                    "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status,
                    response.content_type,
                    response.body.len(),
                    response.body
                );
                stream
                    .write_all(wire_response.as_bytes())
                    .expect("write scripted response");
            }
        });
        (format!("http://{addr}"), requests, handle)
    }

    #[test]
    fn browserstack_adapter_start_maps_espresso_request_into_durable_handle() {
        let responses = vec![
            ScriptedHttpResponse::json(
                "/app-automate/espresso/v2/app",
                json!({"app_url": "bs://app-1"}),
            ),
            ScriptedHttpResponse::json(
                "/app-automate/espresso/v2/test-suite",
                json!({"test_suite_url": "bs://suite-1"}),
            ),
            ScriptedHttpResponse::json(
                "/app-automate/espresso/v2/build",
                json!({"build_id": "build-1"}),
            ),
        ];
        let (base_url, requests, server) = spawn_browserstack_script_server(responses);
        let workspace = tempfile::tempdir().expect("create artifact directory");
        let app = workspace.path().join("app.apk");
        let test_suite = workspace.path().join("test.apk");
        std::fs::write(&app, b"app").expect("write app fixture");
        std::fs::write(&test_suite, b"test").expect("write test fixture");
        let adapter = BrowserStackProviderAdapter::new(test_client_with_base_url(base_url), 1, 0);
        let engine = mobench_provider::ProviderEngine::new(adapter);
        let request = BrowserStackRunRequest {
            devices: vec!["Google Pixel 7-13.0".to_owned()],
            artifacts: BrowserStackArtifacts::Espresso { app, test_suite },
        };

        let handle = engine
            .start(&request, &ProcessCancellation::default())
            .expect("start BrowserStack adapter")
            .into_handle();
        server.join().expect("join scripted server");

        assert_eq!(handle.platform, BrowserStackPlatform::Espresso);
        assert_eq!(handle.requested_devices, request.devices);
        assert_eq!(handle.app_url, "bs://app-1");
        assert_eq!(handle.test_suite_url.as_deref(), Some("bs://suite-1"));
        assert_eq!(handle.build_id, "build-1");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].contains("Google Pixel 7-13.0"));
    }

    #[test]
    fn browserstack_adapter_collect_resumes_a_durable_handle() {
        let responses = vec![
            ScriptedHttpResponse::json(
                "/app-automate/espresso/v2/builds/build-1",
                json!({
                    "build_id": "build-1",
                    "status": "done",
                    "devices": [{
                        "device": "Google Pixel 7-13.0",
                        "session_id": "session-1",
                        "status": "passed"
                    }]
                }),
            ),
            ScriptedHttpResponse::text(
                "/app-automate/espresso/v2/builds/build-1/sessions/session-1/devicelogs",
                r#"{"function":"crate::bench","samples":[1,2,3]}"#,
            ),
            ScriptedHttpResponse::not_found(
                "/app-automate/builds/build-1/sessions/session-1/appprofiling/v2",
            ),
        ];
        let (base_url, _requests, server) = spawn_browserstack_script_server(responses);
        let engine = mobench_provider::ProviderEngine::new(BrowserStackProviderAdapter::new(
            test_client_with_base_url(base_url),
            1,
            0,
        ));
        let handle = BrowserStackRunHandle {
            platform: BrowserStackPlatform::Espresso,
            requested_devices: vec!["Google Pixel 7-13.0".to_owned()],
            app_url: "bs://app-1".to_owned(),
            test_suite_url: Some("bs://suite-1".to_owned()),
            build_id: "build-1".to_owned(),
        };

        let run = engine
            .collect(
                mobench_provider::StartedRun::from_handle(handle),
                &ProcessCancellation::default(),
            )
            .expect("collect resumed BrowserStack run");
        server.join().expect("join scripted server");

        assert!(run.assessment().is_complete());
        assert_eq!(run.sessions().len(), 1);
        assert_eq!(run.sessions()[0].session_id, "session-1");
        assert_eq!(
            run.sessions()[0].reports[0].benchmark["function"],
            "crate::bench"
        );
        assert_eq!(
            run.sessions()[0].reports[0].observed_device_id,
            "Google Pixel 7-13.0"
        );
    }

    #[test]
    fn browserstack_adapter_cancels_during_poll_sleep() {
        let responses = vec![ScriptedHttpResponse::json(
            "/app-automate/espresso/v2/builds/build-1",
            json!({"build_id": "build-1", "status": "running", "devices": []}),
        )];
        let (base_url, _requests, server) = spawn_browserstack_script_server(responses);
        let engine = mobench_provider::ProviderEngine::new(BrowserStackProviderAdapter::new(
            test_client_with_base_url(base_url),
            30,
            10,
        ));
        let handle = BrowserStackRunHandle {
            platform: BrowserStackPlatform::Espresso,
            requested_devices: vec!["Google Pixel 7-13.0".to_owned()],
            app_url: "bs://app-1".to_owned(),
            test_suite_url: Some("bs://suite-1".to_owned()),
            build_id: "build-1".to_owned(),
        };
        let cancellation = ProcessCancellation::default();
        let request_cancellation = cancellation.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            request_cancellation.cancel();
        });
        let started = Instant::now();

        let error = engine
            .collect(
                mobench_provider::StartedRun::from_handle(handle),
                &cancellation,
            )
            .expect_err("polling should be cancelled");
        canceller.join().expect("join cancellation thread");
        server.join().expect("join scripted server");

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn browserstack_adapter_rejects_requested_device_matrix_drift() {
        let requested = vec!["Google Pixel 7-13.0".to_owned(), "iPhone 14-16".to_owned()];
        let observed = vec![
            DeviceSession {
                device: "Google Pixel 7-13.0".to_owned(),
                session_id: "session-1".to_owned(),
                status: "passed".to_owned(),
                device_logs: None,
            },
            DeviceSession {
                device: "Samsung Galaxy S23-13.0".to_owned(),
                session_id: "session-2".to_owned(),
                status: "passed".to_owned(),
                device_logs: None,
            },
        ];

        let error = reconcile_requested_device_sessions(&requested, &observed)
            .expect_err("provider must reject a drifted device matrix");

        assert!(error.to_string().contains("iPhone 14-16"));
        assert!(error.to_string().contains("Samsung Galaxy S23-13.0"));
    }

    #[test]
    fn browserstack_major_version_selector_preserves_requested_and_observed_identity() {
        let requested = vec!["iPhone 14-16".to_owned()];
        let observed = vec![DeviceSession {
            device: "iPhone 14-16.6".to_owned(),
            session_id: "session-1".to_owned(),
            status: "passed".to_owned(),
            device_logs: None,
        }];

        let reconciled = reconcile_requested_device_sessions(&requested, &observed)
            .expect("major selectors should accept a provider minor version");

        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].requested_device_id, "iPhone 14-16");
        assert_eq!(reconciled[0].observed.device, "iPhone 14-16.6");
    }

    #[test]
    fn browserstack_minor_version_selector_requires_an_exact_version() {
        let requested = vec!["iPhone 14-16.0".to_owned()];
        let observed = vec![DeviceSession {
            device: "iPhone 14-16.6".to_owned(),
            session_id: "session-1".to_owned(),
            status: "passed".to_owned(),
            device_logs: None,
        }];

        let error = reconcile_requested_device_sessions(&requested, &observed)
            .expect_err("minor selectors must not float to another provider version");
        assert!(error.to_string().contains("no compatible observed session"));
    }

    #[test]
    fn browserstack_selector_reconciliation_rejects_ambiguous_observed_sessions() {
        let requested = vec!["iPhone 14-16".to_owned()];
        let observed = vec![
            DeviceSession {
                device: "iPhone 14-16.5".to_owned(),
                session_id: "session-1".to_owned(),
                status: "passed".to_owned(),
                device_logs: None,
            },
            DeviceSession {
                device: "iPhone 14-16.6".to_owned(),
                session_id: "session-2".to_owned(),
                status: "passed".to_owned(),
                device_logs: None,
            },
        ];

        let error = reconcile_requested_device_sessions(&requested, &observed)
            .expect_err("floating selectors must reject ambiguous provider matches");
        assert!(
            error
                .to_string()
                .contains("matched multiple observed sessions")
        );
    }

    #[test]
    fn provider_session_details_bind_the_observed_device_and_os_version() {
        let details = SessionDetails {
            device: "Google Pixel 7".to_owned(),
            os: "android".to_owned(),
            os_version: "13.0".to_owned(),
            duration: Some(201),
        };

        assert_eq!(provider_device_identifier(&details), "Google Pixel 7-13.0");
    }

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
    fn authenticated_asset_downloads_are_limited_to_browserstack_https_hosts() {
        assert!(should_authenticate_asset_url(
            "https://api-cloud.browserstack.com/app-automate/logs/123"
        ));
        assert!(should_authenticate_asset_url(
            "https://app-automate.browserstack.com/sessions/123/logs"
        ));
        assert!(!should_authenticate_asset_url(
            "http://api-cloud.browserstack.com/app-automate/logs/123"
        ));
        assert!(!should_authenticate_asset_url(
            "https://evil.example.com/browserstack/logs"
        ));
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

        let result =
            client.schedule_espresso_run(&["Google Pixel 7-13.0".to_string()], "", "bs://test456");

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

        let result =
            client.schedule_espresso_run(&["Google Pixel 7-13.0".to_string()], "bs://app123", "");

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
        assert_eq!(
            results[0].get("function").unwrap().as_str().unwrap(),
            "sample_fns::fibonacci"
        );
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
        assert_eq!(
            results[0].get("function").unwrap().as_str().unwrap(),
            "test1"
        );
        assert_eq!(
            results[1].get("function").unwrap().as_str().unwrap(),
            "test2"
        );
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No benchmark results")
        );
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
        assert_eq!(
            results[0].get("function").unwrap().as_str().unwrap(),
            "test1"
        );
    }

    #[test]
    fn build_status_conversion_from_response() {
        let response = BuildStatusResponse {
            build_id: "test123".to_string(),
            status: "done".to_string(),
            duration: Some(120),
            devices: Some(vec![DeviceSessionResponse {
                device: "Google Pixel 7-13.0".to_string(),
                session_id: "session123".to_string(),
                status: "passed".to_string(),
                device_logs: Some("https://example.com/logs".to_string()),
            }]),
        };

        let status: BuildStatus = response.into();
        assert_eq!(status.build_id, "test123");
        assert_eq!(status.status, "done");
        assert_eq!(status.duration, Some(120));
        assert_eq!(status.devices.len(), 1);
        assert_eq!(status.devices[0].device, "Google Pixel 7-13.0");
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
    fn live_espresso_build_fixture_preserves_observed_device_identity() {
        let value: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/browserstack/espresso-build-passed.json"
        ))
        .expect("valid captured Espresso build fixture");

        let status = build_status_from_value(value).expect("parse Espresso build fixture");

        assert_eq!(status.devices.len(), 1);
        assert_eq!(status.devices[0].device, "Google Pixel 7-13.0");
        assert_eq!(
            status.devices[0].session_id,
            "d7cee5734ae754a811beb3aacc1baba30e810232"
        );
    }

    #[test]
    fn live_xcuitest_build_fixture_reconciles_major_selector_with_observed_minor() {
        let value: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/browserstack/xcuitest-build-passed.json"
        ))
        .expect("valid captured XCUITest build fixture");
        let status = build_status_from_value(value).expect("parse XCUITest build fixture");

        let reconciled =
            reconcile_requested_device_sessions(&["iPhone 14-16".to_owned()], &status.devices)
                .expect("reconcile captured XCUITest build fixture");

        assert_eq!(reconciled[0].requested_device_id, "iPhone 14-16");
        assert_eq!(reconciled[0].observed.device, "iPhone 14-16.6");
        assert_eq!(
            reconciled[0].observed.session_id,
            "916636d2022ee39a8598ed338354af2bcd6c806f"
        );
    }

    #[test]
    fn public_poll_build_completion_errors_on_terminal_failure_status() {
        let payload = json!({
            "build_id": "build123",
            "status": "failed",
            "devices": [{
                "device": "Google Pixel 8-14.0",
                "sessionId": "session123",
                "status": "failed"
            }]
        });
        let (base_url, handle) = spawn_browserstack_path_json_server(
            "/app-automate/espresso/v2/builds/build123",
            payload,
        );
        let client = test_client_with_base_url(base_url);

        let error = client
            .poll_build_completion("build123", "espresso", 1, 1)
            .expect_err("public poll should preserve failure-status errors");
        handle.join().expect("join test server");

        assert!(error.to_string().contains("failed with status: failed"));
    }

    #[test]
    fn internal_poll_can_return_terminal_failure_status_for_log_fetching() {
        let payload = json!({
            "build_id": "build123",
            "status": "failed",
            "devices": [{
                "device": "Google Pixel 8-14.0",
                "sessionId": "session123",
                "status": "failed"
            }]
        });
        let (base_url, handle) = spawn_browserstack_path_json_server(
            "/app-automate/espresso/v2/builds/build123",
            payload,
        );
        let client = test_client_with_base_url(base_url);

        let status = client
            .poll_build_completion_with_terminal_failures("build123", "espresso", 1, 1, true)
            .expect("internal poll should allow log-fetching from failed builds");
        handle.join().expect("join test server");

        assert_eq!(status.status, "failed");
        assert_eq!(status.devices[0].session_id, "session123");
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

    #[test]
    fn parse_app_profiling_v2_response_extracts_memory_and_cpu() {
        let metrics = parse_app_profiling_v2_response(&json!({
            "metadata": {
                "device": "iPhone 15",
                "os_version": "17"
            },
            "data": {
                "units": {
                    "cpu": "%",
                    "mem": "MB"
                },
                "org.world.app": {
                    "status": "success",
                    "metrics": {
                        "cpu": {
                            "avg": 5.06,
                            "max": 12.52
                        },
                        "mem": {
                            "avg": 169.45,
                            "max": 243.57
                        }
                    }
                }
            }
        }))
        .expect("parse v2");

        assert_eq!(metrics.sample_count, 1);
        let cpu = metrics.cpu.expect("cpu");
        assert!((cpu.average_percent - 5.06).abs() < 0.001);
        assert!((cpu.peak_percent - 12.52).abs() < 0.001);

        let memory = metrics.memory.expect("memory");
        assert!((memory.average_mb - 169.45).abs() < 0.001);
        assert!((memory.peak_mb - 243.57).abs() < 0.001);
    }

    #[test]
    fn build_request_serializes_with_app_profiling_enabled() {
        let request = BuildRequest {
            app: "bs://app".into(),
            test_suite: "bs://suite".into(),
            devices: vec!["Google Pixel 8-14.0".into()],
            device_logs: true,
            disable_animations: true,
            build_name: Some("mobench".into()),
            app_profiling: true,
            idle_timeout: ESPRESSO_IDLE_TIMEOUT_SECS,
        };

        let value = serde_json::to_value(&request).expect("serialize build request");
        assert_eq!(value["appProfiling"], true);
        assert_eq!(value["idleTimeout"], ESPRESSO_IDLE_TIMEOUT_SECS);
    }

    #[test]
    fn xcuitest_build_request_serializes_with_app_profiling_enabled() {
        let request = XcuitestBuildRequest {
            app: "bs://app".into(),
            test_suite: "bs://suite".into(),
            devices: vec!["iPhone 15-17".into()],
            device_logs: true,
            build_name: Some("mobench".into()),
            only_testing: Some(vec!["BenchRunnerUITests/test".into()]),
            app_profiling: true,
        };

        let value = serde_json::to_value(&request).expect("serialize xcuitest build request");
        assert_eq!(value["appProfiling"], true);
    }

    #[test]
    fn extract_benchmark_results_handles_ios_markers() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        // Simulate iOS XCUITest logs with BENCH_REPORT_JSON_START/END markers
        let logs = r#"
2026-01-20 12:34:56.789 BenchRunner[1234:5678] Starting benchmark...
2026-01-20 12:34:57.123 BenchRunner[1234:5678] BENCH_REPORT_JSON_START
2026-01-20 12:34:57.124 BenchRunner[1234:5678] {"function": "sample_fns::fibonacci", "samples": [{"duration_ns": 1000000}, {"duration_ns": 1200000}], "mean_ns": 1100000}
2026-01-20 12:34:57.125 BenchRunner[1234:5678] BENCH_REPORT_JSON_END
2026-01-20 12:34:57.200 BenchRunner[1234:5678] Test completed
        "#;

        let results = client.extract_benchmark_results(logs).unwrap();
        assert!(!results.is_empty(), "Should find benchmark results");

        let first = &results[0];
        assert_eq!(
            first.get("function").unwrap().as_str().unwrap(),
            "sample_fns::fibonacci"
        );
        assert_eq!(first.get("mean_ns").unwrap().as_u64().unwrap(), 1100000);
    }

    #[test]
    fn extract_benchmark_results_handles_ios_raw_json() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        // Simulate iOS logs with raw JSON between markers (no log prefix on JSON line)
        let logs = r#"
BENCH_REPORT_JSON_START
{"function": "test_fn", "samples": [{"duration_ns": 500000}], "mean_ns": 500000}
BENCH_REPORT_JSON_END
        "#;

        let results = client.extract_benchmark_results(logs).unwrap();
        assert!(!results.is_empty());
        assert_eq!(
            results[0].get("function").unwrap().as_str().unwrap(),
            "test_fn"
        );
    }

    #[test]
    fn extract_benchmark_results_handles_legacy_uikit_ios_logs() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
2026-05-01 10:00:00.000 BenchRunner[42:7] BENCH_REPORT_JSON_START
2026-05-01 10:00:00.001 BenchRunner[42:7] {"function":"legacy::bench","samples":[{"duration_ns":100},{"duration_ns":200}],"samples_ns":[100,200],"mean_ns":150,"resources":{"platform":"ios","memory_process":"benchmark_app"}}
2026-05-01 10:00:00.002 BenchRunner[42:7] BENCH_REPORT_JSON_END
        "#;

        let results = client.extract_benchmark_results(logs).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["function"], "legacy::bench");
        assert_eq!(results[0]["mean_ns"], 150);
        assert_eq!(results[0]["samples"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn extract_benchmark_results_handles_android_bench_json_marker() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        // Simulate Android logs with BENCH_JSON marker
        let logs = r#"
2026-01-20 12:34:56 I/BenchRunner: Starting benchmark...
2026-01-20 12:34:57 I/BenchRunner: BENCH_JSON {"spec": {"name": "sample_fns::checksum"}, "samples_ns": [1000, 2000], "function": "sample_fns::checksum"}
2026-01-20 12:34:58 I/BenchRunner: Test completed
        "#;

        let results = client.extract_benchmark_results(logs).unwrap();
        assert!(!results.is_empty());
        assert!(results
            .iter()
            .any(|r| r.get("function").and_then(|f| f.as_str()) == Some("sample_fns::checksum")));
    }

    #[test]
    fn extract_benchmark_results_handles_generated_android_chunk_frames() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
2026-07-15 12:34:56 I/BenchRunner: BENCH_JSON_START
2026-07-15 12:34:56 I/BenchRunner: BENCH_JSON_CHUNK {"spec":{"name":"sample_fns::checksum"},
2026-07-15 12:34:56 I/BenchRunner: BENCH_JSON_CHUNK "samples_ns":[1000,2000],"function":"sample_fns::checksum"}
2026-07-15 12:34:56 I/BenchRunner: BENCH_JSON_END
        "#;

        let results = client.extract_benchmark_results(logs).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["function"], "sample_fns::checksum");
        assert_eq!(results[0]["samples_ns"], json!([1000, 2000]));
    }

    #[test]
    fn extract_benchmark_results_rejects_android_chunk_before_start() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();
        let logs = r#"
I/BenchRunner: BENCH_JSON_CHUNK {"function":"forged","samples_ns":[1]}
{"function":"fallback-must-not-win","samples":[{"duration_ns":1}]}
        "#;

        let error = client.extract_benchmark_results(logs).unwrap_err();

        assert!(error.to_string().contains("before a start marker"));
    }

    #[test]
    fn extract_benchmark_results_rejects_truncated_android_chunk_frame() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();
        let logs = r#"
I/BenchRunner: BENCH_JSON_START
I/BenchRunner: BENCH_JSON_CHUNK {"function":"truncated","samples_ns":[1]}
{"function":"fallback-must-not-win","samples":[{"duration_ns":1}]}
        "#;

        let error = client.extract_benchmark_results(logs).unwrap_err();

        assert!(error.to_string().contains("is incomplete"));
    }

    #[test]
    fn extract_benchmark_results_rejects_oversized_android_frame() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();
        let oversized = "x".repeat(mobench_domain::MAX_ANDROID_BENCH_PAYLOAD_BYTES + 1);
        let logs = format!(
            "I/BenchRunner: BENCH_JSON {{\"function\":\"oversized\",\"payload\":\"{oversized}\"}}"
        );

        let error = client.extract_benchmark_results(&logs).unwrap_err();

        assert!(error.to_string().contains("size limit"));
    }

    #[test]
    fn extract_benchmark_failures_handles_failure_only_logs() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
2026-01-20 12:34:57 E/BenchRunner: BENCH_FAILURE_JSON {"schema_version":1,"platform":"android","function_name":"sample_fns::sleep","kind":"timeout","message":"Timed out","elapsed_ms":30000,"android_exit_info":null}
        "#;

        let failures = client.extract_benchmark_failures(logs).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0].get("kind").and_then(|kind| kind.as_str()),
            Some("timeout")
        );
        assert!(client.extract_benchmark_results(logs).is_err());
    }

    #[test]
    fn extract_benchmark_failures_ignores_heartbeat_and_reads_failure() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let logs = r#"
I/BenchRunner: BENCH_HEARTBEAT_JSON {"schema_version":1,"platform":"android","function_name":"sample_fns::sleep","elapsed_ms":10000}
E/BenchRunner: BENCH_FAILURE_JSON {"schema_version":1,"platform":"android","function_name":"sample_fns::sleep","kind":"worker_exit","message":"worker exited","elapsed_ms":12000,"android_exit_info":{"reason":"low_memory","raw_reason":3}}
        "#;

        let failures = client.extract_benchmark_failures(logs).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0]
                .get("android_exit_info")
                .and_then(|info| info.get("reason"))
                .and_then(|reason| reason.as_str()),
            Some("low_memory")
        );
    }

    #[test]
    fn extract_failures_from_session_artifacts_reads_failure_marker() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();
        let session_json = serde_json::json!({
            "automation_session": {
                "device_log_url": "https://example.invalid/device.log"
            }
        });
        let logs = r#"
I/BenchRunner: BENCH_HEARTBEAT_JSON {"schema_version":1,"platform":"android","function_name":"sample_fns::sleep","elapsed_ms":10000}
E/BenchRunner: BENCH_FAILURE_JSON {"schema_version":1,"platform":"android","device":"Vivo Y21-11.0","function_name":"sample_fns::sleep","kind":"timeout","message":"Timed out","elapsed_ms":30000,"android_exit_info":null}
        "#;

        let failures = client
            .extract_failures_from_session_artifacts(&session_json, |url| {
                assert_eq!(url, "https://example.invalid/device.log");
                Ok(logs.to_string())
            })
            .unwrap();

        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0]
                .get("function_name")
                .and_then(|value| value.as_str()),
            Some("sample_fns::sleep")
        );
        assert_eq!(
            failures[0].get("kind").and_then(|value| value.as_str()),
            Some("timeout")
        );
    }

    #[test]
    fn extract_ios_bench_json_finds_last_occurrence() {
        // Test that we find the last occurrence of markers (in case of multiple runs)
        let logs = r#"
BENCH_REPORT_JSON_START
{"function": "first_run", "samples": []}
BENCH_REPORT_JSON_END
Some other logs
BENCH_REPORT_JSON_START
{"function": "second_run", "samples": []}
BENCH_REPORT_JSON_END
        "#;

        let result = BrowserStackClient::extract_ios_bench_json(logs);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().get("function").unwrap().as_str().unwrap(),
            "second_run"
        );
    }

    #[test]
    fn extract_balanced_json_handles_nested_objects() {
        let input = r#"{"outer": {"inner": {"value": 42}}, "extra": "text"} more stuff"#;
        let result = BrowserStackClient::extract_balanced_json(input);
        assert!(result.is_some());
        let json = result.unwrap();
        assert!(json.contains("outer"));
        assert!(json.contains("inner"));
        assert!(!json.contains("more stuff"));
    }

    #[test]
    fn extract_balanced_json_handles_strings_with_braces() {
        let input = r#"{"message": "Hello {world}"}"#;
        let result = BrowserStackClient::extract_balanced_json(input);
        assert!(result.is_some());
        let json = result.unwrap();
        assert_eq!(json, input);
    }

    #[test]
    fn device_identifier_format() {
        let device = BrowserStackDevice {
            device: "Google Pixel 7".to_string(),
            os: "android".to_string(),
            os_version: "13.0".to_string(),
            available: Some(true),
        };
        assert_eq!(device.identifier(), "Google Pixel 7-13.0");
    }

    #[test]
    fn validate_device_spec_exact_match() {
        let devices = vec![
            BrowserStackDevice {
                device: "Google Pixel 7".to_string(),
                os: "android".to_string(),
                os_version: "13.0".to_string(),
                available: Some(true),
            },
            BrowserStackDevice {
                device: "iPhone 14".to_string(),
                os: "ios".to_string(),
                os_version: "16".to_string(),
                available: Some(true),
            },
        ];

        // Exact match should work
        let result = validate_device_spec("Google Pixel 7-13.0", &devices);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Google Pixel 7-13.0");

        // Case-insensitive match
        let result = validate_device_spec("google pixel 7-13.0", &devices);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_device_spec_device_name_only() {
        let devices = vec![BrowserStackDevice {
            device: "Google Pixel 7".to_string(),
            os: "android".to_string(),
            os_version: "13.0".to_string(),
            available: Some(true),
        }];

        // Device name without version should match and return full identifier
        let result = validate_device_spec("Google Pixel 7", &devices);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Google Pixel 7-13.0");
    }

    #[test]
    fn validate_device_spec_suggestions() {
        let devices = vec![
            BrowserStackDevice {
                device: "Google Pixel 7".to_string(),
                os: "android".to_string(),
                os_version: "13.0".to_string(),
                available: Some(true),
            },
            BrowserStackDevice {
                device: "Google Pixel 7 Pro".to_string(),
                os: "android".to_string(),
                os_version: "13.0".to_string(),
                available: Some(true),
            },
        ];

        // Partial match should give suggestions
        let result = validate_device_spec("Pixel 7", &devices);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(!error.suggestions.is_empty());
        assert!(error.suggestions.iter().any(|s| s.contains("Pixel 7")));
    }

    #[test]
    fn validate_device_spec_no_match() {
        let devices = vec![BrowserStackDevice {
            device: "Google Pixel 7".to_string(),
            os: "android".to_string(),
            os_version: "13.0".to_string(),
            available: Some(true),
        }];

        // No match at all
        let result = validate_device_spec("iPhone 14", &devices);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.suggestions.is_empty());
        assert_eq!(error.reason, "No matching device found");
    }

    #[test]
    fn validate_device_spec_wrong_os_version() {
        let devices = vec![
            BrowserStackDevice {
                device: "Google Pixel 7".to_string(),
                os: "android".to_string(),
                os_version: "13.0".to_string(),
                available: Some(true),
            },
            BrowserStackDevice {
                device: "Google Pixel 7".to_string(),
                os: "android".to_string(),
                os_version: "14.0".to_string(),
                available: Some(true),
            },
        ];

        // Wrong OS version should suggest available versions
        let result = validate_device_spec("Google Pixel 7-12.0", &devices);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.reason.contains("OS version"));
        assert!(
            error
                .suggestions
                .contains(&"Google Pixel 7-13.0".to_string())
        );
        assert!(
            error
                .suggestions
                .contains(&"Google Pixel 7-14.0".to_string())
        );
    }

    #[test]
    fn validate_device_spec_limits_suggestions_to_three() {
        let devices = vec![
            BrowserStackDevice {
                device: "Google Pixel 6".to_string(),
                os: "android".to_string(),
                os_version: "12.0".to_string(),
                available: Some(true),
            },
            BrowserStackDevice {
                device: "Google Pixel 7".to_string(),
                os: "android".to_string(),
                os_version: "13.0".to_string(),
                available: Some(true),
            },
            BrowserStackDevice {
                device: "Google Pixel 7 Pro".to_string(),
                os: "android".to_string(),
                os_version: "13.0".to_string(),
                available: Some(true),
            },
            BrowserStackDevice {
                device: "Google Pixel 8".to_string(),
                os: "android".to_string(),
                os_version: "14.0".to_string(),
                available: Some(true),
            },
            BrowserStackDevice {
                device: "Google Pixel 8 Pro".to_string(),
                os: "android".to_string(),
                os_version: "14.0".to_string(),
                available: Some(true),
            },
        ];

        // Should limit to 3 suggestions
        let result = validate_device_spec("Pixel", &devices);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(
            error.suggestions.len() <= 3,
            "Should have at most 3 suggestions, got {}",
            error.suggestions.len()
        );
    }

    #[test]
    fn format_credentials_error_both_missing() {
        let error = format_credentials_error(true, true);
        assert!(error.contains("BrowserStack credentials not configured"));
        assert!(error.contains("BROWSERSTACK_USERNAME"));
        assert!(error.contains("BROWSERSTACK_ACCESS_KEY"));
        assert!(error.contains(".env.local"));
        assert!(error.contains("bench-config.toml"));
        assert!(error.contains("https://app-automate.browserstack.com/"));
    }

    #[test]
    fn format_credentials_error_includes_all_methods() {
        let error = format_credentials_error(true, false);
        // Should always include all three methods regardless of what's missing
        assert!(error.contains("Environment variables"));
        assert!(error.contains("Config file"));
        assert!(error.contains(".env.local"));
    }

    #[test]
    fn parse_device_list_array_format() {
        let json = serde_json::json!([
            {
                "device": "Google Pixel 7",
                "os": "android",
                "os_version": "13.0"
            },
            {
                "device": "iPhone 14",
                "os": "ios",
                "os_version": "16"
            }
        ]);

        let devices = parse_device_list(json, "espresso").unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].device, "Google Pixel 7");
        assert_eq!(devices[1].device, "iPhone 14");
    }

    #[test]
    fn device_discovery_uses_unified_inventory_and_filters_by_os() {
        let payload = json!([
            {
                "device": "Google Pixel 8",
                "os": "ANDROID",
                "os_version": "14.0",
                "available": true
            },
            {
                "device": "iPhone 15",
                "os": "iOS",
                "os_version": "17",
                "available": true
            }
        ]);
        let (base_url, paths, handle) = spawn_browserstack_json_server(payload);
        let client = test_client_with_base_url(base_url);

        let espresso = client.list_espresso_devices();
        let xcuitest = client.list_xcuitest_devices();
        let all = client.list_all_devices();

        handle.join().expect("join test server");

        let espresso = espresso.expect("fetch Android devices from unified inventory");
        let xcuitest = xcuitest.expect("fetch iOS devices from unified inventory");
        let all = all.expect("fetch all devices from unified inventory");

        assert_eq!(
            espresso
                .iter()
                .map(BrowserStackDevice::identifier)
                .collect::<Vec<_>>(),
            vec!["Google Pixel 8-14.0".to_string()]
        );
        assert_eq!(
            xcuitest
                .iter()
                .map(BrowserStackDevice::identifier)
                .collect::<Vec<_>>(),
            vec!["iPhone 15-17".to_string()]
        );
        assert_eq!(
            all.iter()
                .map(BrowserStackDevice::identifier)
                .collect::<Vec<_>>(),
            vec![
                "Google Pixel 8-14.0".to_string(),
                "iPhone 15-17".to_string()
            ]
        );

        let paths = paths.lock().unwrap().clone();
        assert_eq!(
            paths,
            vec![
                "/app-automate/devices.json".to_string(),
                "/app-automate/devices.json".to_string(),
                "/app-automate/devices.json".to_string(),
            ]
        );
    }

    #[test]
    fn extract_results_from_session_artifacts_prefers_bench_report_json() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let session_json = json!({
            "bench_report_url": "https://example.com/bench-report.json",
            "device_logs_url": "https://example.com/device.log",
        });

        let (results, metrics) = client
            .extract_results_from_session_artifacts(&session_json, |url| match url {
                "https://example.com/bench-report.json" => Ok(json!({
                    "spec": {
                        "name": "bench_hash",
                        "iterations": 2,
                        "warmup": 1
                    },
                    "samples": [
                        {"duration_ns": 1000},
                        {"duration_ns": 2000}
                    ]
                })
                .to_string()),
                "https://example.com/device.log" => Ok("no benchmark markers here".to_string()),
                other => Err(anyhow!("unexpected artifact url: {other}")),
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get("function").and_then(|v| v.as_str()),
            Some("bench_hash")
        );
        assert_eq!(
            results[0]
                .get("samples")
                .and_then(|v| v.as_array())
                .map(std::vec::Vec::len),
            Some(2)
        );
        assert_eq!(
            results[0].get("mean_ns").and_then(|v| v.as_u64()),
            Some(1500)
        );
        assert_eq!(metrics.sample_count, 0);
    }

    #[test]
    fn benchmark_normalization_handles_extreme_samples_without_overflow() {
        let normalized = BrowserStackClient::normalize_benchmark_value(json!({
            "function": "bench_extreme",
            "samples_ns": [u64::MAX - 1, u64::MAX]
        }))
        .expect("normalize extreme benchmark samples");

        assert_eq!(
            normalized.get("mean_ns").and_then(Value::as_u64),
            Some(u64::MAX - 1)
        );
        assert_eq!(
            normalized.get("median_ns").and_then(Value::as_u64),
            Some(u64::MAX - 1)
        );
        assert_eq!(
            normalized.get("p95_ns").and_then(Value::as_u64),
            Some(u64::MAX)
        );
    }

    #[test]
    fn extract_results_from_session_artifacts_falls_back_to_ios_log_markers() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let session_json = json!({
            "device_logs_url": "https://example.com/device.log"
        });

        let (results, _) = client
            .extract_results_from_session_artifacts(&session_json, |url| match url {
                "https://example.com/device.log" => Ok(
                    r#"
                    2026-01-20 12:34:57 BenchRunner[1:2] BENCH_REPORT_JSON_START
                    2026-01-20 12:34:57 BenchRunner[1:2] {"spec":{"name":"bench_ios"},"samples_ns":[1000,2000,3000]}
                    2026-01-20 12:34:57 BenchRunner[1:2] BENCH_REPORT_JSON_END
                    "#
                    .to_string(),
                ),
                other => Err(anyhow!("unexpected artifact url: {other}")),
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get("function").and_then(|v| v.as_str()),
            Some("bench_ios")
        );
        assert_eq!(
            results[0].get("p95_ns").and_then(|v| v.as_u64()),
            Some(3000)
        );
    }

    #[test]
    fn extract_results_from_session_artifacts_falls_back_to_android_bench_json_logs() {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "user".into(),
                access_key: "key".into(),
            },
            None,
        )
        .unwrap();

        let session_json = json!({
            "instrumentation_log_url": "https://example.com/instrumentation.log"
        });

        let (results, _) = client
            .extract_results_from_session_artifacts(&session_json, |url| match url {
                "https://example.com/instrumentation.log" => Ok(
                    r#"
                    2026-01-20 12:34:57 I/BenchRunner: BENCH_JSON {"spec":{"name":"bench_android","iterations":2,"warmup":1},"samples_ns":[10,20]}
                    "#
                    .to_string(),
                ),
                other => Err(anyhow!("unexpected artifact url: {other}")),
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get("function").and_then(|v| v.as_str()),
            Some("bench_android")
        );
        assert_eq!(results[0].get("mean_ns").and_then(|v| v.as_u64()), Some(15));
        assert_eq!(
            results[0]
                .get("samples")
                .and_then(|v| v.as_array())
                .map(std::vec::Vec::len),
            Some(2)
        );
    }

    #[test]
    fn browserstack_completeness_rejects_one_of_ten_partial_collection() {
        let expected = (0..10)
            .map(|index| DeviceSession {
                device: format!("device-{index}"),
                session_id: format!("session-{index}"),
                status: "passed".to_string(),
                device_logs: None,
            })
            .collect::<Vec<_>>();
        let collected = vec![CollectedBrowserStackSession {
            session_id: "session-0".to_string(),
            benchmark_results: vec![json!({
                "function": "sample_fns::fibonacci",
                "samples_ns": [10]
            })],
            benchmark_failures: Vec::new(),
            performance_metrics: PerformanceMetrics::default(),
        }];

        let error = classify_browserstack_result_completeness(&expected, collected)
            .expect_err("one result must not make a ten-device run successful");

        assert!(error.to_string().contains("missing collected sessions"));
        assert!(error.to_string().contains("session-9"));
    }

    #[test]
    fn browserstack_completeness_rejects_mixed_passed_and_failed_sessions() {
        let expected = vec![
            DeviceSession {
                device: "passed-device".to_string(),
                session_id: "passed-session".to_string(),
                status: "passed".to_string(),
                device_logs: None,
            },
            DeviceSession {
                device: "failed-device".to_string(),
                session_id: "failed-session".to_string(),
                status: "failed".to_string(),
                device_logs: None,
            },
        ];
        let result = json!({"function": "sample_fns::fibonacci", "samples_ns": [10]});
        let collected = vec![
            CollectedBrowserStackSession {
                session_id: "passed-session".to_string(),
                benchmark_results: vec![result.clone()],
                benchmark_failures: Vec::new(),
                performance_metrics: PerformanceMetrics::default(),
            },
            CollectedBrowserStackSession {
                session_id: "failed-session".to_string(),
                benchmark_results: vec![result],
                benchmark_failures: vec![json!({
                    "kind": "timeout",
                    "message": "benchmark exceeded deadline"
                })],
                performance_metrics: PerformanceMetrics::default(),
            },
        ];

        let error = classify_browserstack_result_completeness(&expected, collected)
            .expect_err("a failed session must make the run incomplete");

        assert!(error.to_string().contains("non-passed sessions"));
        assert!(error.to_string().contains("failed-device"));
        assert!(error.to_string().contains("timeout"));
    }

    #[test]
    fn browserstack_completeness_rejects_passed_session_without_result() {
        let expected = vec![DeviceSession {
            device: "resultless-device".to_string(),
            session_id: "resultless-session".to_string(),
            status: "passed".to_string(),
            device_logs: None,
        }];
        let collected = vec![CollectedBrowserStackSession {
            session_id: "resultless-session".to_string(),
            benchmark_results: Vec::new(),
            benchmark_failures: vec![json!({
                "function_name": "sample_fns::fibonacci",
                "kind": "timeout",
                "message": "benchmark exceeded deadline"
            })],
            performance_metrics: PerformanceMetrics::default(),
        }];

        let error = classify_browserstack_result_completeness(&expected, collected)
            .expect_err("a passed session without a validated result is incomplete");

        assert!(error.to_string().contains("result-less sessions"));
        assert!(error.to_string().contains("resultless-device"));
        assert!(error.to_string().contains("sample_fns::fibonacci"));
        assert!(error.to_string().contains("timeout"));
    }

    #[test]
    fn browserstack_completeness_rejects_duplicate_collected_session() {
        let expected = vec![DeviceSession {
            device: "duplicate-device".to_string(),
            session_id: "duplicate-session".to_string(),
            status: "passed".to_string(),
            device_logs: None,
        }];
        let result = json!({"function": "sample_fns::fibonacci", "samples_ns": [10]});
        let collected = (0..2)
            .map(|_| CollectedBrowserStackSession {
                session_id: "duplicate-session".to_string(),
                benchmark_results: vec![result.clone()],
                benchmark_failures: Vec::new(),
                performance_metrics: PerformanceMetrics::default(),
            })
            .collect();

        let error = classify_browserstack_result_completeness(&expected, collected)
            .expect_err("duplicate collection records are ambiguous");

        assert!(error.to_string().contains("duplicate collected session"));
        assert!(error.to_string().contains("duplicate-session"));
    }

    #[test]
    fn browserstack_completeness_rejects_multiple_results_for_one_session() {
        let expected = vec![DeviceSession {
            device: "duplicate-result-device".to_string(),
            session_id: "duplicate-result-session".to_string(),
            status: "passed".to_string(),
            device_logs: None,
        }];
        let collected = vec![CollectedBrowserStackSession {
            session_id: "duplicate-result-session".to_string(),
            benchmark_results: vec![
                json!({"function": "first", "samples_ns": [10]}),
                json!({"function": "second", "samples_ns": [20]}),
            ],
            benchmark_failures: Vec::new(),
            performance_metrics: PerformanceMetrics::default(),
        }];

        let error = classify_browserstack_result_completeness(&expected, collected)
            .expect_err("one session must produce exactly one validated result");

        assert!(error.to_string().contains("duplicate benchmark results"));
        assert!(error.to_string().contains("got 2"));
    }

    #[test]
    fn browserstack_completeness_rejects_duplicate_expected_device() {
        let expected = (0..2)
            .map(|index| DeviceSession {
                device: "same-device".to_string(),
                session_id: format!("session-{index}"),
                status: "passed".to_string(),
                device_logs: None,
            })
            .collect::<Vec<_>>();
        let collected = (0..2)
            .map(|index| CollectedBrowserStackSession {
                session_id: format!("session-{index}"),
                benchmark_results: vec![json!({
                    "function": "sample_fns::fibonacci",
                    "samples_ns": [10]
                })],
                benchmark_failures: Vec::new(),
                performance_metrics: PerformanceMetrics::default(),
            })
            .collect();

        let error = classify_browserstack_result_completeness(&expected, collected)
            .expect_err("duplicate device labels would overwrite public results");

        assert!(error.to_string().contains("duplicate expected device"));
        assert!(error.to_string().contains("same-device"));
    }

    #[test]
    fn browserstack_completeness_rejects_build_without_expected_sessions() {
        let error = classify_browserstack_result_completeness(&[], Vec::new())
            .expect_err("a terminal build without sessions is incomplete");

        assert!(error.to_string().contains("reported no device sessions"));
    }

    #[test]
    fn browserstack_completeness_accepts_ten_passed_sessions_with_one_result_each() {
        let expected = (0..10)
            .map(|index| DeviceSession {
                device: format!("device-{index}"),
                session_id: format!("session-{index}"),
                status: "passed".to_string(),
                device_logs: None,
            })
            .collect::<Vec<_>>();
        let collected = (0..10)
            .rev()
            .map(|index| CollectedBrowserStackSession {
                session_id: format!("session-{index}"),
                benchmark_results: vec![json!({
                    "function": "sample_fns::fibonacci",
                    "samples_ns": [index + 1]
                })],
                benchmark_failures: Vec::new(),
                performance_metrics: PerformanceMetrics::default(),
            })
            .collect();

        let (results, performance) =
            classify_browserstack_result_completeness(&expected, collected)
                .expect("all expected sessions are complete");

        assert_eq!(results.len(), 10);
        assert_eq!(results["device-9"][0]["samples_ns"], json!([10]));
        assert!(performance.is_empty());
    }
}
