use anyhow::{Context, Result, anyhow};
use reqwest::blocking::multipart::Form;
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

mod recovery;

type BrowserStackResults = (
    std::collections::HashMap<String, Vec<Value>>,
    std::collections::HashMap<String, PerformanceMetrics>,
);
use std::path::Path;
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

            match status.status.to_lowercase().as_str() {
                "done" | "passed" | "completed" => return Ok(status),
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

        Ok(String::from_utf8_lossy(&bytes).into_owned())
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

        // Also look for Android-style BENCH_JSON marker
        let bench_json_marker = "BENCH_JSON ";
        for line in logs.lines() {
            if let Some(idx) = line.find(bench_json_marker) {
                let json_part = &line[idx + bench_json_marker.len()..];
                if let Ok(json) = serde_json::from_str::<Value>(json_part) {
                    Self::extend_unique_results(
                        &mut results,
                        Self::normalize_benchmark_values(json),
                    );
                }
            }
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
        fetch_text: F,
    ) -> Result<(Vec<Value>, PerformanceMetrics)>
    where
        F: FnMut(&str) -> Result<String>,
    {
        let recovered = recovery::recover_from_session_artifacts(
            session_json,
            fetch_text,
            |contents| self.extract_benchmark_results_from_artifact(contents),
            |contents| self.extract_performance_metrics(contents),
        )?;
        Ok((recovered.benchmark_results, recovered.performance_metrics))
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
        let timeout = timeout_secs.unwrap_or(300);
        let poll_interval = poll_interval_secs.unwrap_or(5);

        println!(
            "Waiting for build {} to complete (timeout: {}s, poll: {}s)...",
            build_id, timeout, poll_interval
        );
        let build_status =
            self.poll_build_completion(build_id, platform, timeout, poll_interval)?;

        println!("Build completed with status: {}", build_status.status);
        println!(
            "Fetching results from {} device(s)...",
            build_status.devices.len()
        );

        let mut benchmark_results = std::collections::HashMap::new();
        let mut performance_metrics = std::collections::HashMap::new();

        for device in &build_status.devices {
            println!(
                "  Fetching logs for {} (session: {})...",
                device.device, device.session_id
            );

            let mut device_benchmark_results: Option<Vec<Value>> = None;
            let mut device_performance_metrics = PerformanceMetrics::default();

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

            if device_benchmark_results.is_none() {
                match self
                    .get_session_json(build_id, &device.session_id, platform)
                    .and_then(|session_json| {
                        self.extract_results_from_session_artifacts(&session_json, |url| {
                            self.download_text_url(url)
                        })
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

            if let Some(results) = device_benchmark_results {
                benchmark_results.insert(device.device.clone(), results);
            }
            if device_performance_metrics.sample_count > 0 {
                performance_metrics.insert(device.device.clone(), device_performance_metrics);
            }
        }

        if benchmark_results.is_empty() {
            Err(anyhow!("No benchmark results found from any device"))
        } else {
            Ok((benchmark_results, performance_metrics))
        }
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
        let stats = Self::compute_sample_stats(&samples);
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

    fn compute_sample_stats(samples: &[u64]) -> Option<NormalizedSampleStats> {
        if samples.is_empty() {
            return None;
        }

        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let len = sorted.len();
        let mean_ns =
            (sorted.iter().map(|value| *value as u128).sum::<u128>() / len as u128) as u64;
        let median_ns = if len % 2 == 1 {
            sorted[len / 2]
        } else {
            let lower = sorted[(len / 2) - 1];
            let upper = sorted[len / 2];
            (lower + upper) / 2
        };
        let p95_ns = sorted[Self::percentile_index(len, 0.95)];

        Some(NormalizedSampleStats {
            mean_ns,
            median_ns,
            p95_ns,
            min_ns: sorted[0],
            max_ns: sorted[len - 1],
        })
    }

    fn percentile_index(len: usize, percentile: f64) -> usize {
        if len == 0 {
            return 0;
        }
        let rank = (percentile * len as f64).ceil() as usize;
        let index = rank.saturating_sub(1);
        index.min(len - 1)
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

#[derive(Debug, Clone, Copy)]
struct NormalizedSampleStats {
    mean_ns: u64,
    median_ns: u64,
    p95_ns: u64,
    min_ns: u64,
    max_ns: u64,
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
                .unwrap_or("unknown")
                .to_string();
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
                            device: device_name.clone(),
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
}
