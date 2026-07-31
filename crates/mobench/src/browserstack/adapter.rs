//! BrowserStack adapter for the provider execution seam.

use anyhow::anyhow;
use mobench_process::ProcessCancellation;
use mobench_provider::{AdapterRun, ProviderAdapter};

use super::{
    BrowserStackAdapterError, BrowserStackArtifacts, BrowserStackClient, BrowserStackPlatform,
    BrowserStackReport, BrowserStackRunHandle, BrowserStackRunRequest,
};

/// BrowserStack adapter at the provider seam.
#[derive(Debug, Clone)]
pub(crate) struct BrowserStackProviderAdapter {
    client: BrowserStackClient,
    timeout_secs: u64,
    poll_interval_secs: u64,
}

impl BrowserStackProviderAdapter {
    pub(crate) const fn new(
        client: BrowserStackClient,
        timeout_secs: u64,
        poll_interval_secs: u64,
    ) -> Self {
        Self {
            client,
            timeout_secs,
            poll_interval_secs,
        }
    }
}

impl ProviderAdapter for BrowserStackProviderAdapter {
    type Request = BrowserStackRunRequest;
    type Handle = BrowserStackRunHandle;
    type Report = BrowserStackReport;
    type Error = BrowserStackAdapterError;

    fn start(
        &self,
        request: &Self::Request,
        cancellation: &ProcessCancellation,
    ) -> std::result::Result<Self::Handle, Self::Error> {
        if cancellation.is_cancelled() {
            return Err(anyhow!("BrowserStack start was cancelled").into());
        }

        match &request.artifacts {
            BrowserStackArtifacts::Espresso { app, test_suite } => {
                let app_upload = self
                    .client
                    .upload_espresso_app(app)
                    .map_err(BrowserStackAdapterError::from_anyhow)?;
                if cancellation.is_cancelled() {
                    return Err(anyhow!("BrowserStack start was cancelled after app upload").into());
                }
                let test_upload = self
                    .client
                    .upload_espresso_test_suite(test_suite)
                    .map_err(BrowserStackAdapterError::from_anyhow)?;
                let run = self
                    .client
                    .schedule_espresso_run(
                        &request.devices,
                        &app_upload.app_url,
                        &test_upload.test_suite_url,
                    )
                    .map_err(BrowserStackAdapterError::from_anyhow)?;
                Ok(BrowserStackRunHandle {
                    platform: BrowserStackPlatform::Espresso,
                    requested_devices: request.devices.clone(),
                    app_url: app_upload.app_url,
                    test_suite_url: Some(test_upload.test_suite_url),
                    build_id: run.build_id,
                })
            }
            BrowserStackArtifacts::XcuiTest { app, test_suite } => {
                let app_upload = self
                    .client
                    .upload_xcuitest_app(app)
                    .map_err(BrowserStackAdapterError::from_anyhow)?;
                if cancellation.is_cancelled() {
                    return Err(anyhow!("BrowserStack start was cancelled after app upload").into());
                }
                let test_upload = self
                    .client
                    .upload_xcuitest_test_suite(test_suite)
                    .map_err(BrowserStackAdapterError::from_anyhow)?;
                let run = self
                    .client
                    .schedule_xcuitest_run(
                        &request.devices,
                        &app_upload.app_url,
                        &test_upload.test_suite_url,
                    )
                    .map_err(BrowserStackAdapterError::from_anyhow)?;
                Ok(BrowserStackRunHandle {
                    platform: BrowserStackPlatform::XcuiTest,
                    requested_devices: request.devices.clone(),
                    app_url: app_upload.app_url,
                    test_suite_url: Some(test_upload.test_suite_url),
                    build_id: run.build_id,
                })
            }
        }
    }

    fn collect(
        &self,
        handle: &Self::Handle,
        cancellation: &ProcessCancellation,
    ) -> std::result::Result<AdapterRun<Self::Report>, Self::Error> {
        self.client
            .wait_and_collect_adapter_run(
                &handle.build_id,
                handle.platform,
                Some(&handle.requested_devices),
                self.timeout_secs,
                self.poll_interval_secs,
                cancellation,
            )
            .map_err(BrowserStackAdapterError::from_anyhow)
    }

    fn cancel(
        &self,
        _handle: &Self::Handle,
        _cancellation: &ProcessCancellation,
    ) -> std::result::Result<(), Self::Error> {
        // BrowserStack does not expose a stable cross-runner cancellation call
        // yet. Cancellation stops local polling immediately; the durable build
        // handle remains usable for delayed collection and diagnostics.
        Ok(())
    }
}
