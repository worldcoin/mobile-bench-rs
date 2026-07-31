//! BrowserStack Automate transport for browser-hosted benchmarks.
//!
//! This module is deliberately separate from the App Automate client in
//! [`crate::browserstack`]. It speaks the W3C WebDriver protocol directly and
//! does not build, upload, or schedule native applications.

#![allow(dead_code)] // The CLI wiring lands in a separate parity slice.

use crate::browserstack::BrowserStackAuth;
use anyhow::{Context, Result, anyhow, bail};
use mobench_process::ProcessCancellation;
use mobench_sdk::{BenchSpec, RunnerReport};
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::io::Read;
use std::time::{Duration, Instant};

const DEFAULT_WEBDRIVER_URL: &str = "https://hub.browserstack.com/wd/hub";
const USER_AGENT: &str = "mobench/0.2";
// BrowserStack may take longer than a normal API request to allocate a session,
// especially while a release matrix is starting several browsers at once.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OPERATION_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_READY_POLL: Duration = Duration::from_millis(250);
const DEFAULT_BENCHMARK_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_BENCHMARK_POLL: Duration = Duration::from_secs(1);
const MAX_WEBDRIVER_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ERROR_MESSAGE_CHARS: usize = 1024;
const MAX_CAPABILITY_CHARS: usize = 256;
const MAX_BENCHMARK_URL_CHARS: usize = 8 * 1024;

/// Browser and operating-system capabilities for one Automate session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BrowserEnvironment {
    pub browser: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

impl BrowserEnvironment {
    pub(crate) fn desktop(
        browser: impl Into<String>,
        browser_version: Option<String>,
        os: impl Into<String>,
        os_version: impl Into<String>,
    ) -> Self {
        Self {
            browser: browser.into(),
            browser_version,
            os: Some(os.into()),
            os_version: Some(os_version.into()),
            device: None,
        }
    }

    pub(crate) fn mobile(
        browser: impl Into<String>,
        device: impl Into<String>,
        os_version: impl Into<String>,
    ) -> Self {
        Self {
            browser: browser.into(),
            browser_version: None,
            os: None,
            os_version: Some(os_version.into()),
            device: Some(device.into()),
        }
    }

    fn validate(&self) -> Result<()> {
        validate_capability("browser", &self.browser)?;
        validate_optional_capability("browser version", self.browser_version.as_deref())?;
        validate_optional_capability("OS", self.os.as_deref())?;
        validate_optional_capability("OS version", self.os_version.as_deref())?;
        validate_optional_capability("device", self.device.as_deref())?;
        if self.device.is_some() && self.os_version.is_none() {
            bail!("mobile BrowserStack environments require an OS version");
        }
        Ok(())
    }
}

/// BrowserStack metadata and Local-tunnel binding for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AutomateSessionOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    #[serde(default)]
    pub local: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_identifier: Option<String>,
}

impl AutomateSessionOptions {
    fn validate(&self) -> Result<()> {
        validate_optional_capability("project name", self.project_name.as_deref())?;
        validate_optional_capability("build name", self.build_name.as_deref())?;
        validate_optional_capability("session name", self.session_name.as_deref())?;
        validate_optional_capability("local identifier", self.local_identifier.as_deref())?;
        if self.local_identifier.is_some() && !self.local {
            bail!("a BrowserStack Local identifier requires local=true");
        }
        Ok(())
    }
}

/// Durable handle returned by BrowserStack when a browser session starts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct AutomateSession {
    pub session_id: String,
    #[serde(default)]
    pub capabilities: Value,
}

/// Complete request for one browser benchmark session.
#[derive(Debug, Clone)]
pub(crate) struct AutomateRunRequest {
    pub environment: BrowserEnvironment,
    pub options: AutomateSessionOptions,
    pub url: String,
    pub spec: BenchSpec,
    pub script_timeout: Duration,
    pub page_load_timeout: Duration,
    pub ready_timeout: Duration,
}

impl AutomateRunRequest {
    pub(crate) fn new(
        environment: BrowserEnvironment,
        url: impl Into<String>,
        spec: BenchSpec,
    ) -> Self {
        Self {
            environment,
            options: AutomateSessionOptions::default(),
            url: url.into(),
            spec,
            script_timeout: DEFAULT_BENCHMARK_TIMEOUT,
            page_load_timeout: Duration::from_secs(60),
            ready_timeout: DEFAULT_READY_TIMEOUT,
        }
    }

    fn validate(&self) -> Result<()> {
        self.environment.validate()?;
        self.options.validate()?;
        validate_nonzero_timeout("script", self.script_timeout)?;
        validate_nonzero_timeout("page-load", self.page_load_timeout)?;
        validate_nonzero_timeout("ready", self.ready_timeout)?;
        validate_capability("benchmark name", &self.spec.name)?;
        parse_benchmark_url(&self.url)?;
        Ok(())
    }
}

/// Minimal, bounded W3C WebDriver client.
#[derive(Clone)]
pub(crate) struct BrowserStackAutomateClient {
    http: Client,
    auth: BrowserStackAuth,
    webdriver_url: String,
}

impl std::fmt::Debug for BrowserStackAutomateClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserStackAutomateClient")
            .field("auth", &self.auth)
            .field("webdriver_url", &self.webdriver_url)
            .finish_non_exhaustive()
    }
}

impl BrowserStackAutomateClient {
    pub(crate) fn new(auth: BrowserStackAuth) -> Result<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            // Session allocation and browser execution may exceed reqwest's
            // default request timeout. The WebDriver page/script deadlines,
            // bounded polling loops, and cancellation token are authoritative.
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .context("building BrowserStack Automate HTTP client")?;
        Ok(Self {
            http,
            auth,
            webdriver_url: DEFAULT_WEBDRIVER_URL.to_owned(),
        })
    }

    #[cfg(test)]
    fn with_webdriver_url(mut self, webdriver_url: impl Into<String>) -> Self {
        self.webdriver_url = webdriver_url.into();
        self
    }

    pub(crate) fn create_session(
        &self,
        environment: &BrowserEnvironment,
        options: &AutomateSessionOptions,
    ) -> Result<AutomateSession> {
        environment.validate()?;
        options.validate()?;

        let mut bstack = serde_json::Map::new();
        insert_option(&mut bstack, "os", environment.os.as_ref());
        insert_option(&mut bstack, "osVersion", environment.os_version.as_ref());
        if let Some(device) = &environment.device {
            bstack.insert("deviceName".into(), Value::String(device.clone()));
            bstack.insert("realMobile".into(), Value::String("true".into()));
        }
        insert_option(&mut bstack, "projectName", options.project_name.as_ref());
        insert_option(&mut bstack, "buildName", options.build_name.as_ref());
        insert_option(&mut bstack, "sessionName", options.session_name.as_ref());
        if options.local {
            bstack.insert("local".into(), Value::String("true".into()));
        }
        insert_option(
            &mut bstack,
            "localIdentifier",
            options.local_identifier.as_ref(),
        );

        let mut always_match = serde_json::Map::new();
        always_match.insert(
            "browserName".into(),
            Value::String(environment.browser.clone()),
        );
        insert_option(
            &mut always_match,
            "browserVersion",
            environment.browser_version.as_ref(),
        );
        always_match.insert("bstack:options".into(), Value::Object(bstack));

        let response: Value = self.send_json(
            self.authenticated(self.http.post(self.endpoint("session"))),
            Some(&json!({
                "capabilities": {
                    "alwaysMatch": always_match,
                    "firstMatch": [{}]
                }
            })),
            "creating BrowserStack Automate session",
        )?;
        parse_session_response(response)
    }

    pub(crate) fn set_timeouts(
        &self,
        session_id: &str,
        script_timeout: Duration,
        page_load_timeout: Duration,
    ) -> Result<()> {
        validate_nonzero_timeout("script", script_timeout)?;
        validate_nonzero_timeout("page-load", page_load_timeout)?;
        let path = session_path(session_id, "timeouts")?;
        let _: Value = self.send_json(
            self.authenticated(self.http.post(self.endpoint(&path))),
            Some(&json!({
                "script": duration_millis(script_timeout)?,
                "pageLoad": duration_millis(page_load_timeout)?
            })),
            "configuring BrowserStack Automate timeouts",
        )?;
        Ok(())
    }

    pub(crate) fn navigate(&self, session_id: &str, url: &str) -> Result<()> {
        let url = parse_benchmark_url(url)?;
        let path = session_path(session_id, "url")?;
        let _: Value = self.send_json(
            self.authenticated(self.http.post(self.endpoint(&path))),
            Some(&json!({ "url": url.as_str() })),
            "navigating BrowserStack Automate session",
        )?;
        Ok(())
    }

    pub(crate) fn execute_script(
        &self,
        session_id: &str,
        script: &str,
        args: &[Value],
    ) -> Result<Value> {
        self.execute(session_id, "execute/sync", script, args)
    }

    #[allow(dead_code)]
    pub(crate) fn execute_async_script(
        &self,
        session_id: &str,
        script: &str,
        args: &[Value],
    ) -> Result<Value> {
        self.execute(session_id, "execute/async", script, args)
    }

    pub(crate) fn wait_until_ready(
        &self,
        session_id: &str,
        timeout: Duration,
        cancellation: &ProcessCancellation,
    ) -> Result<()> {
        self.wait_until_ready_with(session_id, timeout, DEFAULT_READY_POLL, cancellation)
    }

    fn wait_until_ready_with(
        &self,
        session_id: &str,
        timeout: Duration,
        poll_interval: Duration,
        cancellation: &ProcessCancellation,
    ) -> Result<()> {
        validate_nonzero_timeout("ready", timeout)?;
        let started = Instant::now();
        loop {
            check_cancelled(cancellation)?;
            let ready = self.execute_script(
                session_id,
                "return Boolean(window.mobench && typeof window.mobench.run === 'function');",
                &[],
            )?;
            if ready.as_bool() == Some(true) {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                bail!(
                    "timed out after {} ms waiting for the mobench web harness",
                    timeout.as_millis()
                );
            }
            sleep_cancellable(poll_interval, cancellation)?;
        }
    }

    pub(crate) fn run_benchmark(
        &self,
        session_id: &str,
        spec: &Value,
        timeout: Duration,
        cancellation: &ProcessCancellation,
    ) -> Result<Value> {
        self.run_benchmark_with_poll(
            session_id,
            spec,
            timeout,
            DEFAULT_BENCHMARK_POLL,
            cancellation,
        )
    }

    fn run_benchmark_with_poll(
        &self,
        session_id: &str,
        spec: &Value,
        timeout: Duration,
        poll_interval: Duration,
        cancellation: &ProcessCancellation,
    ) -> Result<Value> {
        validate_nonzero_timeout("benchmark", timeout)?;
        check_cancelled(cancellation)?;
        self.execute_script(
            session_id,
            r#"
const spec = arguments[0];
window.__mobenchRunState = { status: "running" };
setTimeout(() => {
  Promise.resolve()
    .then(() => window.mobench.run(spec))
    .then((result) => {
      window.__mobenchRunState = { status: "ok", result };
    })
    .catch((error) => {
      window.__mobenchRunState = {
        status: "error",
        error: error && error.message ? String(error.message) : String(error)
      };
    });
}, 0);
return true;
"#,
            std::slice::from_ref(spec),
        )?;

        let started = Instant::now();
        loop {
            check_cancelled(cancellation)?;
            let state = match self.execute_script(
                session_id,
                "return window.__mobenchRunState || { status: 'missing' };",
                &[],
            ) {
                Ok(state) => state,
                Err(error) if is_browser_busy_error(&error) && started.elapsed() < timeout => {
                    sleep_cancellable(poll_interval, cancellation)?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            match state.get("status").and_then(Value::as_str) {
                Some("ok") => {
                    return state
                        .get("result")
                        .cloned()
                        .ok_or_else(|| anyhow!("BrowserStack web benchmark returned no result"));
                }
                Some("error") => {
                    let message = bounded_printable(
                        state
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("web benchmark failed without an error message"),
                    );
                    bail!("BrowserStack web benchmark failed: {message}");
                }
                Some("running") => {}
                Some(status) => {
                    bail!(
                        "BrowserStack web benchmark returned invalid status {}",
                        bounded_printable(status)
                    )
                }
                None => bail!("BrowserStack web benchmark returned no status"),
            }
            if started.elapsed() >= timeout {
                bail!(
                    "timed out after {} ms waiting for the BrowserStack web benchmark",
                    timeout.as_millis()
                );
            }
            sleep_cancellable(poll_interval, cancellation)?;
        }
    }

    /// Runs a complete session and always attempts status reporting and quit.
    pub(crate) fn run_mobench_session(
        &self,
        request: &AutomateRunRequest,
        cancellation: &ProcessCancellation,
    ) -> Result<RunnerReport> {
        request.validate()?;
        check_cancelled(cancellation)?;
        let session = self.create_session(&request.environment, &request.options)?;
        let result = (|| {
            self.set_timeouts(
                &session.session_id,
                request.script_timeout,
                request.page_load_timeout,
            )?;
            self.navigate(&session.session_id, &request.url)?;
            self.wait_until_ready(&session.session_id, request.ready_timeout, cancellation)?;
            let spec =
                serde_json::to_value(&request.spec).context("serializing browser BenchSpec")?;
            let value = self.run_benchmark(
                &session.session_id,
                &spec,
                request.script_timeout,
                cancellation,
            )?;
            serde_json::from_value(value).context("decoding browser RunnerReport")
        })();

        self.finish_session(&session.session_id, result)
    }

    fn finish_session(
        &self,
        session_id: &str,
        result: Result<RunnerReport>,
    ) -> Result<RunnerReport> {
        let passed = result.is_ok();
        let status_result = self.set_session_status(
            session_id,
            passed,
            if passed {
                "mobench benchmark completed"
            } else {
                "mobench benchmark failed"
            },
        );
        let quit_result = self.quit(session_id);

        match result {
            Ok(report) => {
                status_result.context("marking successful BrowserStack session")?;
                quit_result.context("closing successful BrowserStack session")?;
                Ok(report)
            }
            Err(run_error) => {
                if let Err(quit_error) = quit_result {
                    return Err(run_error.context(format!(
                        "also failed to close BrowserStack session: {}",
                        bounded_printable(&quit_error.to_string())
                    )));
                }
                // Status failure must not replace the original benchmark
                // failure. It is intentionally ignored after the best effort.
                let _ = status_result;
                Err(run_error)
            }
        }
    }

    pub(crate) fn set_session_status(
        &self,
        session_id: &str,
        passed: bool,
        reason: &str,
    ) -> Result<()> {
        validate_capability("session status reason", reason)?;
        let status = if passed { "passed" } else { "failed" };
        let executor = json!({
            "action": "setSessionStatus",
            "arguments": { "status": status, "reason": reason }
        });
        let script = format!("browserstack_executor: {executor}");
        let _: Value = self.execute_script(session_id, &script, &[])?;
        Ok(())
    }

    pub(crate) fn quit(&self, session_id: &str) -> Result<()> {
        let path = session_path(session_id, "")?;
        let response = self
            .authenticated(self.http.delete(self.endpoint(&path)))
            .send()
            .context("quitting BrowserStack Automate session")?;
        self.parse_response::<Value>(response, "quitting BrowserStack Automate session")?;
        Ok(())
    }

    fn execute(
        &self,
        session_id: &str,
        endpoint: &str,
        script: &str,
        args: &[Value],
    ) -> Result<Value> {
        if script.trim().is_empty() {
            bail!("WebDriver script must not be empty");
        }
        let path = session_path(session_id, endpoint)?;
        self.send_json(
            self.authenticated(self.http.post(self.endpoint(&path))),
            Some(&json!({ "script": script, "args": args })),
            "executing JavaScript in BrowserStack Automate session",
        )
    }

    fn send_json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        body: Option<&Value>,
        context: &str,
    ) -> Result<T> {
        let request = if let Some(body) = body {
            request.json(body)
        } else {
            request
        };
        let response = request.send().with_context(|| context.to_owned())?;
        self.parse_response(response, context)
    }

    fn authenticated(&self, request: RequestBuilder) -> RequestBuilder {
        request.basic_auth(&self.auth.username, Some(&self.auth.access_key))
    }

    fn parse_response<T: DeserializeOwned>(&self, response: Response, context: &str) -> Result<T> {
        parse_webdriver_response(response, context)
            .map_err(|error| anyhow!(redact_secret(&error.to_string(), &self.auth.access_key)))
    }

    fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.webdriver_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

fn insert_option(map: &mut serde_json::Map<String, Value>, key: &str, value: Option<&String>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), Value::String(value.clone()));
    }
}

fn parse_session_response(response: Value) -> Result<AutomateSession> {
    let value = response.get("value").unwrap_or(&response);
    let session_id = value
        .get("sessionId")
        .or_else(|| response.get("sessionId"))
        .and_then(Value::as_str)
        .context("BrowserStack WebDriver response did not contain a session id")?;
    validate_session_id(session_id)?;
    Ok(AutomateSession {
        session_id: session_id.to_owned(),
        capabilities: value
            .get("capabilities")
            .cloned()
            .unwrap_or_else(|| json!({})),
    })
}

fn parse_webdriver_response<T: DeserializeOwned>(
    mut response: Response,
    context: &str,
) -> Result<T> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|bytes| bytes > MAX_WEBDRIVER_RESPONSE_BYTES as u64)
    {
        bail!("{context}: BrowserStack response exceeded the size limit");
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take((MAX_WEBDRIVER_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("{context}: reading BrowserStack response"))?;
    if bytes.len() > MAX_WEBDRIVER_RESPONSE_BYTES {
        bail!("{context}: BrowserStack response exceeded the size limit");
    }
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("{context}: parsing BrowserStack response JSON"))?;
    let webdriver_value = value.get("value").unwrap_or(&value);
    if !status.is_success() {
        bail!(
            "{context}: BrowserStack returned HTTP {status}: {}",
            webdriver_error_message(webdriver_value)
        );
    }
    if webdriver_value.get("error").is_some() {
        bail!(
            "{context}: BrowserStack WebDriver error: {}",
            webdriver_error_message(webdriver_value)
        );
    }
    serde_json::from_value(webdriver_value.clone())
        .with_context(|| format!("{context}: decoding BrowserStack response"))
}

fn webdriver_error_message(value: &Value) -> String {
    bounded_printable(
        value
            .get("message")
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("unknown WebDriver error"),
    )
}

fn bounded_printable(raw: &str) -> String {
    let mut message = raw
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_ERROR_MESSAGE_CHARS)
        .collect::<String>();
    if raw.chars().count() > MAX_ERROR_MESSAGE_CHARS {
        message.push('…');
    }
    message
}

fn session_path(session_id: &str, suffix: &str) -> Result<String> {
    validate_session_id(session_id)?;
    if suffix.is_empty() {
        Ok(format!("session/{session_id}"))
    } else {
        Ok(format!("session/{session_id}/{}", suffix.trim_matches('/')))
    }
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty()
        || session_id.len() > MAX_CAPABILITY_CHARS
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("BrowserStack returned an invalid WebDriver session id");
    }
    Ok(())
}

fn validate_capability(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_CAPABILITY_CHARS
        || value.chars().any(char::is_control)
    {
        bail!(
            "{label} must be a non-empty printable value of at most {MAX_CAPABILITY_CHARS} characters"
        );
    }
    Ok(())
}

fn validate_optional_capability(label: &str, value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        validate_capability(label, value)?;
    }
    Ok(())
}

fn validate_nonzero_timeout(label: &str, timeout: Duration) -> Result<()> {
    if timeout.is_zero() {
        bail!("{label} timeout must be greater than zero");
    }
    if timeout > MAX_OPERATION_TIMEOUT {
        bail!(
            "{label} timeout must not exceed {} seconds",
            MAX_OPERATION_TIMEOUT.as_secs()
        );
    }
    if duration_millis(timeout)? == 0 {
        bail!("{label} timeout must be at least one millisecond");
    }
    Ok(())
}

fn duration_millis(duration: Duration) -> Result<u64> {
    u64::try_from(duration.as_millis()).context("WebDriver timeout is too large")
}

fn parse_benchmark_url(url: &str) -> Result<reqwest::Url> {
    if url.chars().count() > MAX_BENCHMARK_URL_CHARS {
        bail!("web benchmark URL exceeds the size limit");
    }
    let parsed = reqwest::Url::parse(url).context("parsing web benchmark URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("web benchmark URL must use http or https");
    }
    Ok(parsed)
}

fn redact_secret(message: &str, secret: &str) -> String {
    if secret.is_empty() {
        message.to_owned()
    } else {
        message.replace(secret, "[REDACTED]")
    }
}

fn is_browser_busy_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("Did not get any response for atom execution")
        || message.contains("Timed out receiving message from renderer")
}

fn check_cancelled(cancellation: &ProcessCancellation) -> Result<()> {
    if cancellation.is_cancelled() {
        bail!("BrowserStack Automate execution was cancelled");
    }
    Ok(())
}

fn sleep_cancellable(duration: Duration, cancellation: &ProcessCancellation) -> Result<()> {
    const SLICE: Duration = Duration::from_millis(50);
    let started = Instant::now();
    while started.elapsed() < duration {
        check_cancelled(cancellation)?;
        std::thread::sleep(SLICE.min(duration.saturating_sub(started.elapsed())));
    }
    check_cancelled(cancellation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[derive(Debug)]
    struct RecordedRequest {
        method: String,
        path: String,
        body: Value,
        authorization: Option<String>,
    }

    fn client(webdriver_url: impl Into<String>) -> BrowserStackAutomateClient {
        BrowserStackAutomateClient::new(BrowserStackAuth {
            username: "user".into(),
            access_key: "super-secret-key".into(),
        })
        .unwrap()
        .with_webdriver_url(webdriver_url)
    }

    fn spawn_server(
        responses: Vec<(u16, Value)>,
    ) -> (
        String,
        Arc<Mutex<Vec<RecordedRequest>>>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock WebDriver");
        let addr = listener.local_addr().unwrap();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let recorded_thread = Arc::clone(&recorded);
        let mut responses = VecDeque::from(responses);
        let request_count = responses.len();
        let handle = thread::spawn(move || {
            for _ in 0..request_count {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 4096];
                let header_end = loop {
                    let read = stream.read(&mut chunk).expect("read request");
                    assert_ne!(read, 0, "request ended before headers");
                    bytes.extend_from_slice(&chunk[..read]);
                    if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let mut request_line = headers.lines().next().unwrap().split_whitespace();
                let method = request_line.next().unwrap_or_default().to_owned();
                let path = request_line.next().unwrap_or_default().to_owned();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let authorization = headers.lines().find_map(|line| {
                    line.to_ascii_lowercase()
                        .starts_with("authorization:")
                        .then(|| line.to_owned())
                });
                while bytes.len().saturating_sub(header_end) < content_length {
                    let read = stream.read(&mut chunk).expect("read body");
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let body = if content_length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[header_end..header_end + content_length])
                        .expect("valid request JSON")
                };
                recorded_thread.lock().unwrap().push(RecordedRequest {
                    method,
                    path,
                    body,
                    authorization,
                });

                let (status, body) = responses.pop_front().unwrap();
                let reason = if status < 400 { "OK" } else { "Error" };
                let body = body.to_string();
                write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .expect("write response");
            }
        });
        (format!("http://{addr}/wd/hub"), recorded, handle)
    }

    fn ok(value: impl Into<Value>) -> (u16, Value) {
        (200, json!({ "value": value.into() }))
    }

    #[test]
    fn credentials_are_redacted_from_debug_output() {
        let debug = format!(
            "{:?}",
            BrowserStackAuth {
                username: "user".into(),
                access_key: "super-secret-key".into(),
            }
        );
        assert!(debug.contains("user"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-key"));
    }

    #[test]
    fn creates_w3c_desktop_and_local_mobile_sessions() {
        let (url, requests, server) = spawn_server(vec![
            ok(json!({"sessionId":"desktop-1","capabilities":{}})),
            ok(json!({"sessionId":"mobile_1","capabilities":{}})),
        ]);
        let client = client(url);
        client
            .create_session(
                &BrowserEnvironment::desktop("chrome", Some("latest".into()), "OS X", "Sequoia"),
                &AutomateSessionOptions {
                    project_name: Some("mobench".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        client
            .create_session(
                &BrowserEnvironment::mobile("safari", "iPhone 16 Pro Max", "18"),
                &AutomateSessionOptions {
                    local: true,
                    local_identifier: Some("run-1".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        server.join().unwrap();

        let requests = requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .all(|request| request.authorization.is_some())
        );
        assert_eq!(
            requests[0].body["capabilities"]["alwaysMatch"]["browserVersion"],
            "latest"
        );
        let mobile = &requests[1].body["capabilities"]["alwaysMatch"]["bstack:options"];
        assert_eq!(mobile["deviceName"], "iPhone 16 Pro Max");
        assert_eq!(mobile["local"], "true");
        assert_eq!(mobile["localIdentifier"], "run-1");
    }

    #[test]
    fn configures_timeouts_polls_and_cleans_up() {
        let report = json!({
            "spec":{"name":"sample::bench","iterations":1,"warmup":0},
            "samples":[{"iteration":0,"duration_ns":42}],
            "summary":{
                "mean_ns":42.0,"median_ns":42.0,"min_ns":42,"max_ns":42,
                "std_dev_ns":0.0,"p95_ns":42.0
            }
        });
        let (url, requests, server) = spawn_server(vec![
            ok(json!({"sessionId":"session-123","capabilities":{}})),
            ok(Value::Null),
            ok(Value::Null),
            ok(true),
            ok(true),
            ok(json!({"status":"running"})),
            ok(json!({"status":"ok","result":report})),
            ok(Value::Null),
            ok(Value::Null),
        ]);
        let client = client(url);
        let mut request = AutomateRunRequest::new(
            BrowserEnvironment::desktop("chrome", None, "Windows", "11"),
            "https://bench.example.test/",
            BenchSpec {
                name: "sample::bench".into(),
                iterations: 1,
                warmup: 0,
            },
        );
        request.script_timeout = Duration::from_secs(1);
        request.page_load_timeout = Duration::from_secs(1);
        request.ready_timeout = Duration::from_secs(1);
        let report = client
            .run_mobench_session(&request, &ProcessCancellation::default())
            .unwrap();
        server.join().unwrap();

        assert_eq!(report.spec.name, "sample::bench");
        let requests = requests.lock().unwrap();
        assert_eq!(requests[1].body["script"], 1000);
        assert_eq!(requests.last().unwrap().method, "DELETE");
        assert_eq!(requests.last().unwrap().path, "/wd/hub/session/session-123");
    }

    #[test]
    fn failure_is_marked_and_session_is_still_closed() {
        let (url, requests, server) = spawn_server(vec![
            ok(json!({"sessionId":"session-123","capabilities":{}})),
            ok(Value::Null),
            ok(Value::Null),
            ok(true),
            ok(true),
            ok(json!({"status":"error","error":"benchmark exploded"})),
            ok(Value::Null),
            ok(Value::Null),
        ]);
        let client = client(url);
        let mut request = AutomateRunRequest::new(
            BrowserEnvironment::desktop("safari", None, "OS X", "Sequoia"),
            "https://bench.example.test/",
            BenchSpec {
                name: "sample::bench".into(),
                iterations: 1,
                warmup: 0,
            },
        );
        request.script_timeout = Duration::from_secs(1);
        let error = client
            .run_mobench_session(&request, &ProcessCancellation::default())
            .unwrap_err();
        server.join().unwrap();
        assert!(error.to_string().contains("benchmark exploded"));
        let requests = requests.lock().unwrap();
        assert!(
            requests[6].body["script"]
                .as_str()
                .unwrap()
                .contains(r#""status":"failed""#)
        );
        assert_eq!(requests[7].method, "DELETE");
    }

    #[test]
    fn busy_browser_errors_are_retried() {
        let (url, _, server) = spawn_server(vec![
            ok(true),
            (
                500,
                json!({"value":{
                    "error":"unknown error",
                    "message":"Did not get any response for atom execution after 130165ms"
                }}),
            ),
            ok(json!({"status":"ok","result":{"answer":42}})),
        ]);
        let result = client(url)
            .run_benchmark_with_poll(
                "session-123",
                &json!({"name":"sample::bench"}),
                Duration::from_secs(1),
                Duration::ZERO,
                &ProcessCancellation::default(),
            )
            .unwrap();
        server.join().unwrap();
        assert_eq!(result["answer"], 42);
    }

    #[test]
    fn cancellation_stops_polling() {
        let cancellation = ProcessCancellation::default();
        cancellation.cancel();
        let error = client("http://127.0.0.1:9/wd/hub")
            .run_benchmark(
                "session-123",
                &json!({}),
                Duration::from_secs(1),
                &cancellation,
            )
            .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn validates_local_binding_urls_sessions_and_timeouts() {
        assert!(
            AutomateSessionOptions {
                local_identifier: Some("run-1".into()),
                ..Default::default()
            }
            .validate()
            .unwrap_err()
            .to_string()
            .contains("local=true")
        );
        assert!(
            client("http://127.0.0.1:9/wd/hub")
                .navigate("../unsafe", "https://example.test")
                .unwrap_err()
                .to_string()
                .contains("invalid WebDriver session id")
        );
        assert!(parse_benchmark_url("file:///tmp/index.html").is_err());
        assert!(validate_nonzero_timeout("script", Duration::ZERO).is_err());
    }

    #[test]
    fn webdriver_errors_are_bounded_and_control_characters_removed() {
        let raw = format!("secret\r\n{}", "x".repeat(MAX_ERROR_MESSAGE_CHARS + 50));
        let message = webdriver_error_message(&json!({"message":raw}));
        assert!(!message.contains('\r'));
        assert!(!message.contains('\n'));
        assert!(message.ends_with('…'));
        assert!(message.chars().count() <= MAX_ERROR_MESSAGE_CHARS + 1);
    }

    #[test]
    fn webdriver_errors_redact_the_access_key() {
        let (url, _, server) = spawn_server(vec![(
            500,
            json!({"value":{
                "error":"unknown error",
                "message":"credential super-secret-key was rejected"
            }}),
        )]);
        let error = client(url)
            .execute_script("session-123", "return true;", &[])
            .unwrap_err();
        server.join().unwrap();
        assert!(error.to_string().contains("[REDACTED]"));
        assert!(!error.to_string().contains("super-secret-key"));
    }

    #[test]
    fn rejects_oversized_webdriver_response_from_content_length() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_WEBDRIVER_RESPONSE_BYTES + 1
            )
            .unwrap();
        });
        let error = client(format!("http://{addr}/wd/hub"))
            .execute_script("session-123", "return true;", &[])
            .unwrap_err();
        server.join().unwrap();
        assert!(error.to_string().contains("exceeded the size limit"));
    }
}
