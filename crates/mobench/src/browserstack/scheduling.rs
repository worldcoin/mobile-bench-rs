//! BrowserStack build scheduling for Espresso and XCUITest.

use anyhow::{Context, Result, anyhow};

use super::{
    BrowserStackClient, BuildRequest, BuildResponse, ESPRESSO_IDLE_TIMEOUT_SECS, ScheduledRun,
    XcuitestBuildRequest, parse_response,
};

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
}
