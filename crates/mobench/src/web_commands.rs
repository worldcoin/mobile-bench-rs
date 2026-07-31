//! Browser-hosted WASM benchmark command orchestration.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::browserstack::BrowserStackAuth;
use crate::browserstack_automate::{
    AutomateRunRequest, AutomateSessionOptions, BrowserEnvironment, BrowserStackAutomateClient,
};
use crate::{resolve_browserstack_credentials, write_file};

#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_run_web(
    url: String,
    function: String,
    iterations: u32,
    warmup: u32,
    browser: String,
    browser_version: Option<String>,
    os: String,
    os_version: String,
    device: Option<String>,
    build_name: Option<String>,
    session_name: Option<String>,
    local_identifier: Option<String>,
    script_timeout_secs: u64,
    page_load_timeout_secs: u64,
    output: PathBuf,
    dry_run: bool,
) -> Result<()> {
    let environment = if let Some(device) = device {
        BrowserEnvironment::mobile(browser, device, os_version)
    } else {
        BrowserEnvironment::desktop(browser, browser_version, os, os_version)
    };
    let local = local_identifier.is_some();
    let spec = mobench_sdk::BenchSpec {
        name: function,
        iterations,
        warmup,
    };
    if dry_run {
        println!("Would run web benchmark at {url}");
        println!("  Environment: {environment:?}");
        println!("  Spec: {spec:?}");
        println!("  BrowserStack Local: {local}");
        println!("  Output: {}", output.display());
        return Ok(());
    }

    let credentials = resolve_browserstack_credentials(None)?;
    let client = BrowserStackAutomateClient::new(BrowserStackAuth {
        username: credentials.username,
        access_key: credentials.access_key,
    })?;
    let mut request = AutomateRunRequest::new(environment, url, spec);
    request.options = AutomateSessionOptions {
        project_name: credentials.project,
        build_name,
        session_name,
        local,
        local_identifier,
    };
    request.script_timeout = Duration::from_secs(script_timeout_secs);
    request.page_load_timeout = Duration::from_secs(page_load_timeout_secs);

    let report =
        client.run_mobench_session(&request, &mobench_process::global_cancellation_token())?;
    let contents = serde_json::to_vec_pretty(&report).context("serializing web RunnerReport")?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating web result directory {}", parent.display()))?;
    }
    write_file(&output, &contents)?;
    println!("Web benchmark completed: {}", output.display());
    Ok(())
}
