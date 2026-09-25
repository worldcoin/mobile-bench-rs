//! BrowserStack build scheduling for Espresso and XCUITest.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use mobench_process::ProcessCancellation;

use super::{
    BrowserStackClient, BrowserStackPlatform, BrowserStackRunHandle, BuildRequest, BuildResponse,
    ESPRESSO_IDLE_TIMEOUT_SECS, ScheduledRun, XcuitestBuildRequest, parse_response,
};

/// BrowserStack's error code when every parallel, including the queue, is taken.
const ALL_PARALLELS_IN_USE: &str = "BROWSERSTACK_ALL_PARALLELS_IN_USE";

/// True when BrowserStack rejected a build because all parallels are in use.
pub(crate) fn is_busy_parallels_error(error: &anyhow::Error) -> bool {
    format!("{error:#}").contains(ALL_PARALLELS_IN_USE)
}

/// Delay before busy-parallel retry `retry` (1-based): 30s doubling to a 5 minute cap, with jitter.
pub(crate) fn busy_parallels_backoff(retry: u8) -> Duration {
    let capped = (30u64 << u32::from(retry.saturating_sub(1)).min(4)).min(300);
    let half = capped / 2;
    // Random half spreads out concurrent waiters so they don't all retry at once.
    let mut bytes = [0u8; 8];
    let jitter = getrandom::fill(&mut bytes)
        .map(|()| u64::from_le_bytes(bytes) % (half + 1))
        .unwrap_or(0);
    Duration::from_secs(half + jitter)
}

/// Runs `schedule`, waiting and retrying up to `retries` times while all BrowserStack parallels are in use.
pub(crate) fn schedule_when_parallels_free<T>(
    retries: u8,
    cancellation: &ProcessCancellation,
    mut wait: impl FnMut(Duration, &ProcessCancellation) -> Result<()>,
    mut schedule: impl FnMut() -> Result<T>,
) -> Result<T> {
    let mut retry = 0u8;
    loop {
        match schedule() {
            Ok(value) => return Ok(value),
            Err(error) if retry < retries && is_busy_parallels_error(&error) => {
                retry += 1;
                let delay = busy_parallels_backoff(retry);
                eprintln!(
                    "Warning: all BrowserStack parallels are in use; retrying schedule {retry}/{retries} in {}s",
                    delay.as_secs()
                );
                wait(delay, cancellation)?;
            }
            Err(error) => return Err(error),
        }
    }
}

impl BrowserStackClient {
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
            app: app_url.to_owned(),
            test_suite: test_suite_url.to_owned(),
            devices: devices.to_vec(),
            device_logs: true,
            disable_animations: true,
            app_profiling: true,
            idle_timeout: ESPRESSO_IDLE_TIMEOUT_SECS,
            build_name: self.project.clone(),
        };
        let response = self
            .http
            .post(self.api("app-automate/espresso/v2/build"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .json(&body)
            .send()
            .context("scheduling BrowserStack Espresso run")?;
        let build: BuildResponse = parse_response(response, "schedule run")?;
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
            app: app_url.to_owned(),
            test_suite: test_suite_url.to_owned(),
            devices: devices.to_vec(),
            device_logs: true,
            app_profiling: true,
            build_name: self.project.clone(),
            only_testing: Some(vec![
                "BenchRunnerUITests/BenchRunnerUITests/testLaunchAndCaptureBenchmarkReport"
                    .to_owned(),
            ]),
        };
        let response = self
            .http
            .post(self.api("app-automate/xcuitest/v2/build"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .json(&body)
            .send()
            .context("scheduling BrowserStack XCUITest run")?;
        let build: BuildResponse = parse_response(response, "schedule run")?;
        Ok(ScheduledRun {
            build_id: build.build_id,
        })
    }

    /// Schedules a fresh build reusing an existing run's uploads, waiting for a free parallel if needed.
    pub(crate) fn reschedule_run(
        &self,
        handle: &BrowserStackRunHandle,
        busy_parallel_retries: u8,
        cancellation: &ProcessCancellation,
    ) -> Result<BrowserStackRunHandle> {
        let test_suite_url = handle.test_suite_url.as_deref().ok_or_else(|| {
            anyhow!(
                "BrowserStack build {} has no test suite to reschedule",
                handle.build_id
            )
        })?;
        let run = schedule_when_parallels_free(
            busy_parallel_retries,
            cancellation,
            super::polling::sleep_cancellable,
            || match handle.platform {
                BrowserStackPlatform::Espresso => self.schedule_espresso_run(
                    &handle.requested_devices,
                    &handle.app_url,
                    test_suite_url,
                ),
                BrowserStackPlatform::XcuiTest => self.schedule_xcuitest_run(
                    &handle.requested_devices,
                    &handle.app_url,
                    test_suite_url,
                ),
            },
        )?;
        Ok(BrowserStackRunHandle {
            build_id: run.build_id,
            ..handle.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn busy_error() -> anyhow::Error {
        anyhow!(
            "BrowserStack API schedule run failed (status 422 Unprocessable Entity): {{\"message\":\"[BROWSERSTACK_ALL_PARALLELS_IN_USE] All parallel tests are currently in use, including the queued tests.\"}}"
        )
    }

    #[test]
    fn busy_parallels_error_matches_only_the_browserstack_code() {
        assert!(is_busy_parallels_error(&busy_error()));
        assert!(is_busy_parallels_error(
            &busy_error().context("scheduling BrowserStack XCUITest run")
        ));
        assert!(!is_busy_parallels_error(&anyhow!(
            "BrowserStack API schedule run failed (status 422 Unprocessable Entity): {{\"message\":\"invalid device\"}}"
        )));
    }

    #[test]
    fn busy_parallels_backoff_doubles_to_a_capped_jittered_delay() {
        for (retry, low, high) in [
            (1, 15, 30),
            (2, 30, 60),
            (3, 60, 120),
            (4, 120, 240),
            (5, 150, 300),
            (255, 150, 300),
        ] {
            for _ in 0..50 {
                let secs = busy_parallels_backoff(retry).as_secs();
                assert!((low..=high).contains(&secs), "retry {retry} waited {secs}s");
            }
        }
    }

    #[test]
    fn schedule_waits_out_busy_parallels_then_succeeds() {
        let calls = Cell::new(0);
        let waits = Cell::new(0);
        let result = schedule_when_parallels_free(
            3,
            &ProcessCancellation::default(),
            |_, _| {
                waits.set(waits.get() + 1);
                Ok(())
            },
            || {
                calls.set(calls.get() + 1);
                if calls.get() < 3 {
                    Err(busy_error())
                } else {
                    Ok("build-1")
                }
            },
        );
        assert_eq!(result.expect("schedules"), "build-1");
        assert_eq!((calls.get(), waits.get()), (3, 2));
    }

    #[test]
    fn schedule_gives_up_after_retries_and_never_retries_other_errors() {
        let calls = Cell::new(0);
        let result: Result<()> = schedule_when_parallels_free(
            2,
            &ProcessCancellation::default(),
            |_, _| Ok(()),
            || {
                calls.set(calls.get() + 1);
                Err(busy_error())
            },
        );
        assert!(is_busy_parallels_error(&result.expect_err("still busy")));
        assert_eq!(calls.get(), 3);

        let calls = Cell::new(0);
        let result: Result<()> = schedule_when_parallels_free(
            5,
            &ProcessCancellation::default(),
            |_, _| panic!("must not wait"),
            || {
                calls.set(calls.get() + 1);
                Err(anyhow!("invalid device"))
            },
        );
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn schedule_stops_when_the_wait_is_cancelled() {
        let calls = Cell::new(0);
        let result: Result<()> = schedule_when_parallels_free(
            5,
            &ProcessCancellation::default(),
            |_, _| Err(anyhow!("cancelled")),
            || {
                calls.set(calls.get() + 1);
                Err(busy_error())
            },
        );
        assert_eq!(result.expect_err("cancelled").to_string(), "cancelled");
        assert_eq!(calls.get(), 1);
    }
}
