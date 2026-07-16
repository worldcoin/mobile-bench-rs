//! Bounded BrowserStack build polling and cooperative cancellation.

use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use mobench_process::ProcessCancellation;

use super::{BrowserStackClient, BuildStatus};

fn sleep_cancellable(duration: Duration, cancellation: &ProcessCancellation) -> Result<()> {
    let started = Instant::now();
    while started.elapsed() < duration {
        if cancellation.is_cancelled() {
            return Err(anyhow!("BrowserStack collection was cancelled"));
        }
        let remaining = duration.saturating_sub(started.elapsed());
        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
    Ok(())
}

impl BrowserStackClient {
    /// Poll for build completion with a bounded deadline.
    #[allow(dead_code)]
    pub fn poll_build_completion(
        &self,
        build_id: &str,
        platform: &str,
        timeout_secs: u64,
        poll_interval_secs: u64,
    ) -> Result<BuildStatus> {
        self.poll_build_completion_with_terminal_failures(
            build_id,
            platform,
            timeout_secs,
            poll_interval_secs,
            false,
        )
    }

    pub(super) fn poll_build_completion_with_terminal_failures(
        &self,
        build_id: &str,
        platform: &str,
        timeout_secs: u64,
        poll_interval_secs: u64,
        allow_terminal_failure_status: bool,
    ) -> Result<BuildStatus> {
        self.poll_build_completion_cancellable(
            build_id,
            platform,
            timeout_secs,
            poll_interval_secs,
            allow_terminal_failure_status,
            &mobench_process::global_cancellation_token(),
        )
    }

    pub(super) fn poll_build_completion_cancellable(
        &self,
        build_id: &str,
        platform: &str,
        timeout_secs: u64,
        poll_interval_secs: u64,
        allow_terminal_failure_status: bool,
        cancellation: &ProcessCancellation,
    ) -> Result<BuildStatus> {
        let start = Instant::now();
        let timeout = Duration::from_secs(timeout_secs);
        let poll_interval = Duration::from_secs(poll_interval_secs);

        loop {
            if cancellation.is_cancelled() {
                return Err(anyhow!("BrowserStack collection was cancelled"));
            }
            let status = match platform {
                "espresso" => self.get_espresso_build_status(build_id)?,
                "xcuitest" => self.get_xcuitest_build_status(build_id)?,
                _ => return Err(anyhow!("unsupported platform: {platform}")),
            };

            match status.status.to_lowercase().as_str() {
                "done" | "passed" | "completed" => return Ok(status),
                "failed" | "error" | "timeout" => {
                    if allow_terminal_failure_status {
                        return Ok(status);
                    }
                    return Err(anyhow!(
                        "Build {build_id} failed with status: {}",
                        status.status
                    ));
                }
                _ => {
                    if start.elapsed() >= timeout {
                        return Err(anyhow!(
                            "Timeout waiting for build {build_id} to complete (waited {timeout_secs} seconds)"
                        ));
                    }
                    sleep_cancellable(poll_interval, cancellation)?;
                }
            }
        }
    }
}
