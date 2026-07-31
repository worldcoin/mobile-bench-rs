//! Terminal BrowserStack evidence collection.
//!
//! This module owns the ordered transition from a durable provider build
//! handle to one adapter run: bounded polling, requested/observed device
//! reconciliation, live-log extraction, artifact fallback, telemetry merge,
//! and deterministic session assembly.

use anyhow::{Result, anyhow};
use mobench_process::ProcessCancellation;
use mobench_provider::{AdapterRun, ProviderRun};
use serde_json::Value;

use super::{
    BrowserStackClient, BrowserStackPlatform, BrowserStackReport, BrowserStackResults,
    CollectedBrowserStackSession, DEFAULT_BROWSERSTACK_FETCH_TIMEOUT_SECS, PerformanceMetrics,
    ReconciledDeviceSession, browserstack_adapter_run_with_bindings,
    completed_browserstack_results, merge_performance_metrics, reconcile_requested_device_sessions,
};

impl BrowserStackClient {
    /// Wait for build completion and fetch all results using the default poll interval.
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
        let run: ProviderRun<BrowserStackReport> = self
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

    pub(super) fn wait_and_collect_adapter_run(
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
            "Waiting for build {build_id} to complete (timeout: {timeout}s, poll: {poll_interval}s)..."
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
            Some(requested) => {
                reconcile_requested_device_sessions(requested, &build_status.devices)?
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

            let mut benchmark_results: Option<Vec<Value>> = None;
            let mut performance = PerformanceMetrics::default();
            let mut failures: Option<Vec<Value>> = None;

            match self.get_device_logs(build_id, &device.session_id, platform) {
                Ok(logs) => {
                    match self.extract_benchmark_results(&logs) {
                        Ok(results) => {
                            println!("    Found {} benchmark result(s)", results.len());
                            benchmark_results = Some(results);
                        }
                        Err(error) => println!("    No benchmark results in live logs: {error}"),
                    }
                    match self.extract_benchmark_failures(&logs) {
                        Ok(found) => {
                            println!("    Found {} benchmark failure marker(s)", found.len());
                            failures = Some(found);
                        }
                        Err(error) => println!("    No benchmark failures in live logs: {error}"),
                    }
                    match self.extract_performance_metrics(&logs) {
                        Ok(metrics) if metrics.sample_count > 0 => {
                            println!(
                                "    Found {} performance metric snapshot(s)",
                                metrics.sample_count
                            );
                            performance = metrics;
                        }
                        Ok(_) => println!("    No performance metrics found in live logs"),
                        Err(error) => {
                            println!("    Warning: Failed to extract performance metrics - {error}")
                        }
                    }
                }
                Err(error) => println!("    Failed to fetch live logs: {error}"),
            }

            let fetch_failure_artifacts = failures.is_none()
                && (benchmark_results.is_none() || !device.status.eq_ignore_ascii_case("passed"));
            if benchmark_results.is_none() || fetch_failure_artifacts {
                match self.get_session_json(build_id, &device.session_id, platform) {
                    Ok(session_json) => {
                        if benchmark_results.is_none() {
                            match self
                                .extract_results_from_session_artifacts(&session_json, |url| {
                                    self.download_text_url(url)
                                }) {
                                Ok((results, metrics)) => {
                                    println!(
                                        "    Found {} benchmark result(s) from session artifacts",
                                        results.len()
                                    );
                                    if performance.sample_count == 0 && metrics.sample_count > 0 {
                                        println!(
                                            "    Found {} performance metric snapshot(s) from session artifacts",
                                            metrics.sample_count
                                        );
                                        performance = metrics;
                                    }
                                    benchmark_results = Some(results);
                                }
                                Err(error) => println!(
                                    "    Warning: Failed to fetch results from session artifacts: {error}"
                                ),
                            }
                        }
                        if fetch_failure_artifacts
                            && let Ok(found) = self
                                .extract_failures_from_session_artifacts(&session_json, |url| {
                                    self.download_text_url(url)
                                })
                        {
                            println!(
                                "    Found {} benchmark failure marker(s) from session artifacts",
                                found.len()
                            );
                            failures = Some(found);
                        }
                    }
                    Err(error) => {
                        println!("    Warning: Failed to fetch session artifacts metadata: {error}")
                    }
                }
            }

            if let Ok(app_profile) = self.get_app_profiling_v2(build_id, &device.session_id)
                && app_profile.sample_count > 0
            {
                println!("    Found App Profiling v2 metrics");
                performance = merge_performance_metrics(Some(performance), Some(app_profile))
                    .unwrap_or_default();
            }

            collected_sessions.push(CollectedBrowserStackSession {
                session_id: device.session_id.clone(),
                benchmark_results: benchmark_results.unwrap_or_default(),
                benchmark_failures: failures.unwrap_or_default(),
                performance_metrics: performance,
            });
        }

        Ok(browserstack_adapter_run_with_bindings(
            &verified_sessions,
            collected_sessions,
        ))
    }
}
