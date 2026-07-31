//! BrowserStack app and test-suite uploads.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::multipart::Form;

use super::{
    AppUpload, BrowserStackClient, TestSuiteUpload, format_file_size, get_file_size, parse_response,
};

impl BrowserStackClient {
    pub fn upload_espresso_app(&self, artifact: &Path) -> Result<AppUpload> {
        if !artifact.exists() {
            return Err(anyhow!("app artifact not found at {artifact:?}"));
        }
        println!(
            "Uploading Android APK ({})...",
            format_file_size(get_file_size(artifact))
        );
        let started = Instant::now();
        let response = self
            .http
            .post(self.api("app-automate/espresso/v2/app"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(Form::new().file("file", artifact)?)
            .send()
            .context("uploading app to BrowserStack")?;
        let result = parse_response(response, "app upload")?;
        println!(
            "  Uploaded Android APK (took {}s)",
            started.elapsed().as_secs()
        );
        Ok(result)
    }

    pub fn upload_espresso_test_suite(&self, artifact: &Path) -> Result<TestSuiteUpload> {
        if !artifact.exists() {
            return Err(anyhow!("test suite artifact not found at {artifact:?}"));
        }
        println!(
            "Uploading Android test APK ({})...",
            format_file_size(get_file_size(artifact))
        );
        let started = Instant::now();
        let response = self
            .http
            .post(self.api("app-automate/espresso/v2/test-suite"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(Form::new().file("file", artifact)?)
            .send()
            .context("uploading test suite to BrowserStack")?;
        let result = parse_response(response, "test suite upload")?;
        println!(
            "  Uploaded Android test APK (took {}s)",
            started.elapsed().as_secs()
        );
        Ok(result)
    }

    pub fn upload_xcuitest_app(&self, artifact: &Path) -> Result<AppUpload> {
        if !artifact.exists() {
            return Err(anyhow!("iOS app artifact not found at {artifact:?}"));
        }
        println!(
            "Uploading iOS app IPA ({})...",
            format_file_size(get_file_size(artifact))
        );
        let started = Instant::now();
        let response = self
            .http
            .post(self.api("app-automate/xcuitest/v2/app"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(Form::new().file("file", artifact)?)
            .send()
            .context("uploading iOS app to BrowserStack")?;
        let result = parse_response(response, "iOS app upload")?;
        println!(
            "  Uploaded iOS app IPA (took {}s)",
            started.elapsed().as_secs()
        );
        Ok(result)
    }

    pub fn upload_xcuitest_test_suite(&self, artifact: &Path) -> Result<TestSuiteUpload> {
        if !artifact.exists() {
            return Err(anyhow!(
                "iOS XCUITest suite artifact not found at {artifact:?}"
            ));
        }
        println!(
            "Uploading iOS XCUITest runner ({})...",
            format_file_size(get_file_size(artifact))
        );
        let started = Instant::now();
        let response = self
            .http
            .post(self.api("app-automate/xcuitest/v2/test-suite"))
            .basic_auth(&self.auth.username, Some(&self.auth.access_key))
            .multipart(Form::new().file("file", artifact)?)
            .send()
            .context("uploading iOS XCUITest suite to BrowserStack")?;
        let result = parse_response(response, "iOS XCUITest suite upload")?;
        println!(
            "  Uploaded iOS XCUITest runner (took {}s)",
            started.elapsed().as_secs()
        );
        Ok(result)
    }
}
