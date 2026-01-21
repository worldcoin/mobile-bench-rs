use anyhow::{Context, Result, anyhow};
use reqwest::blocking::multipart::Form;
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
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
        println!("Uploading Android test APK ({})...", format_file_size(file_size));
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
        println!("Uploading iOS XCUITest runner ({})...", format_file_size(file_size));
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
                "BenchRunnerUITests/BenchRunnerUITests/testLaunchAndCaptureBenchmarkReport".to_string(),
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

    /// List available Android devices for Espresso testing.
    pub fn list_espresso_devices(&self) -> Result<Vec<BrowserStackDevice>> {
        let json = self.get_json("app-automate/espresso/v2/devices")?;
        parse_device_list(json, "espresso")
    }

    /// List available iOS devices for XCUITest testing.
    pub fn list_xcuitest_devices(&self) -> Result<Vec<BrowserStackDevice>> {
        let json = self.get_json("app-automate/xcuitest/v2/devices")?;
        parse_device_list(json, "xcuitest")
    }

    /// List all available devices (both Android and iOS).
    pub fn list_all_devices(&self) -> Result<Vec<BrowserStackDevice>> {
        let mut devices = Vec::new();

        match self.list_espresso_devices() {
            Ok(android_devices) => devices.extend(android_devices),
            Err(e) => {
                eprintln!("Warning: Failed to fetch Android devices: {}", e);
            }
        }

        match self.list_xcuitest_devices() {
            Ok(ios_devices) => devices.extend(ios_devices),
            Err(e) => {
                eprintln!("Warning: Failed to fetch iOS devices: {}", e);
            }
        }

        Ok(devices)
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

    /// Extract benchmark results from device logs
    /// Looks for JSON output matching BenchReport format
    /// Supports both Android (BENCH_JSON) and iOS (BENCH_REPORT_JSON_START/END) formats
    pub fn extract_benchmark_results(&self, logs: &str) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        // First, try iOS-style markers: BENCH_REPORT_JSON_START ... BENCH_REPORT_JSON_END
        if let Some(json) = Self::extract_ios_bench_json(logs) {
            results.push(json);
        }

        // Also look for Android-style BENCH_JSON marker
        let bench_json_marker = "BENCH_JSON ";
        for line in logs.lines() {
            if let Some(idx) = line.find(bench_json_marker) {
                let json_part = &line[idx + bench_json_marker.len()..];
                if let Ok(json) = serde_json::from_str::<Value>(json_part) {
                    if json.get("function").is_some()
                        || json.get("samples").is_some()
                        || json.get("spec").is_some()
                    {
                        results.push(json);
                    }
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
                && (json.get("function").is_some() || json.get("samples").is_some())
            {
                // Avoid duplicates
                if !results
                    .iter()
                    .any(|existing| existing.to_string() == json.to_string())
                {
                    results.push(json);
                }
            }
        }

        if results.is_empty() {
            Err(anyhow!("No benchmark results found in device logs"))
        } else {
            Ok(results)
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
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            if let Ok(json) = serde_json::from_str::<Value>(trimmed) {
                return Some(json);
            }
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
                if let Some(json) = Self::extract_balanced_json(potential_json) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&json) {
                        return Some(parsed);
                    }
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
            if let Some(json) = Self::extract_balanced_json(potential_json) {
                if let Ok(parsed) = serde_json::from_str::<Value>(&json) {
                    return Some(parsed);
                }
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

        Ok(PerformanceMetrics::from_snapshots(snapshots))
    }

    /// Wait for build completion and fetch all results including performance metrics
    ///
    /// Returns both benchmark results and performance metrics
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
                            println!(
                                "    Found {} performance metric snapshot(s)",
                                perf_metrics.sample_count
                            );
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
            .unwrap_or(if context == "xcuitest" { "ios" } else { "android" })
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
        if version_part.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            (device_part.to_lowercase(), Some(version_part.to_lowercase()))
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
            let available_versions: Vec<String> = matching_devices
                .iter()
                .map(|d| d.identifier())
                .collect();

            let mut suggestions = available_versions;
            suggestions.sort();
            suggestions.truncate(3);

            return Err(DeviceValidationError {
                spec: spec.to_string(),
                reason: format!(
                    "OS version '{}' not available for this device",
                    version
                ),
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

            let matches = spec_words.iter().filter(|sw|
                device_words.iter().any(|dw| dw.contains(*sw))
            ).count();

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
    scored_suggestions.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1))
    });

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
        assert!(error.suggestions.contains(&"Google Pixel 7-13.0".to_string()));
        assert!(error.suggestions.contains(&"Google Pixel 7-14.0".to_string()));
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
        assert!(error.suggestions.len() <= 3, "Should have at most 3 suggestions, got {}", error.suggestions.len());
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
}
