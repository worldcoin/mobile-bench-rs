//! BrowserStack Automate transport for web and WebAssembly benchmarks.
//!
//! App Automate uploads native applications and test suites. Web benchmarks
//! instead use the W3C WebDriver protocol to open a generated benchmark page
//! and return its structured result directly to mobench.

use crate::browserstack::BrowserStackAuth;
use anyhow::{Context, Result, anyhow, bail};
use mobench_sdk::{BenchSpec, RunnerReport};
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::io::Read;
use std::time::{Duration, Instant};

const DEFAULT_WEBDRIVER_URL: &str = "https://hub.browserstack.com/wd/hub";
const USER_AGENT: &str = "mobench/0.1";
const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_READY_POLL: Duration = Duration::from_millis(250);
const MAX_WEBDRIVER_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Browser and operating-system capabilities for one Automate session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserEnvironment {
    /// W3C browser name, such as `chrome`, `firefox`, or `safari`.
    pub browser: String,
    /// Optional browser version. Omit it to use BrowserStack's current default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_version: Option<String>,
    /// Desktop operating system, such as `OS X` or `Windows`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// Desktop or mobile operating-system version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    /// Real mobile device name. Omit for desktop browser sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

impl BrowserEnvironment {
    /// Creates a desktop browser environment.
    pub fn desktop(
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

    /// Creates a real mobile browser environment.
    pub fn mobile(
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

/// BrowserStack-specific metadata attached to a W3C session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomateSessionOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    /// Enable BrowserStack Local for a private or loopback benchmark URL.
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
pub struct AutomateSession {
    pub session_id: String,
    #[serde(default)]
    pub capabilities: Value,
}

/// Complete request for one mobench BrowserStack Automate session.
#[derive(Debug, Clone)]
pub struct AutomateRunRequest {
    pub environment: BrowserEnvironment,
    pub options: AutomateSessionOptions,
    pub url: String,
    pub spec: BenchSpec,
    pub script_timeout: Duration,
    pub page_load_timeout: Duration,
}

impl AutomateRunRequest {
    pub fn new(environment: BrowserEnvironment, url: impl Into<String>, spec: BenchSpec) -> Self {
        Self {
            environment,
            options: AutomateSessionOptions::default(),
            url: url.into(),
            spec,
            script_timeout: Duration::from_secs(300),
            page_load_timeout: Duration::from_secs(60),
        }
    }
}

/// Minimal W3C WebDriver client used by the WASM backend.
#[derive(Debug, Clone)]
pub struct BrowserStackAutomateClient {
    http: Client,
    auth: BrowserStackAuth,
    webdriver_url: String,
}

impl BrowserStackAutomateClient {
    pub fn new(auth: BrowserStackAuth) -> Result<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            // BrowserStack session allocation and benchmark scripts can both
            // legitimately take longer than reqwest's blocking-client
            // 30-second default. WebDriver and workflow timeouts remain the
            // explicit bounds; connection establishment stays separately
            // bounded below.
            .timeout(None)
            .connect_timeout(Duration::from_secs(15))
            .build()
            .context("building BrowserStack Automate HTTP client")?;
        Ok(Self {
            http,
            auth,
            webdriver_url: DEFAULT_WEBDRIVER_URL.to_string(),
        })
    }

    #[cfg(test)]
    fn with_webdriver_url(mut self, webdriver_url: impl Into<String>) -> Self {
        self.webdriver_url = webdriver_url.into();
        self
    }

    /// Starts one BrowserStack browser session.
    pub fn create_session(
        &self,
        environment: &BrowserEnvironment,
        options: &AutomateSessionOptions,
    ) -> Result<AutomateSession> {
        environment.validate()?;
        options.validate()?;

        let mut bstack_options = serde_json::Map::new();
        if let Some(os) = &environment.os {
            bstack_options.insert("os".to_string(), Value::String(os.clone()));
        }
        if let Some(os_version) = &environment.os_version {
            bstack_options.insert("osVersion".to_string(), Value::String(os_version.clone()));
        }
        if let Some(device) = &environment.device {
            bstack_options.insert("deviceName".to_string(), Value::String(device.clone()));
            bstack_options.insert("realMobile".to_string(), Value::String("true".to_string()));
        }
        if let Some(project_name) = &options.project_name {
            bstack_options.insert(
                "projectName".to_string(),
                Value::String(project_name.clone()),
            );
        }
        if let Some(build_name) = &options.build_name {
            bstack_options.insert("buildName".to_string(), Value::String(build_name.clone()));
        }
        if let Some(session_name) = &options.session_name {
            bstack_options.insert(
                "sessionName".to_string(),
                Value::String(session_name.clone()),
            );
        }
        if options.local {
            bstack_options.insert("local".to_string(), Value::String("true".to_string()));
        }
        if let Some(local_identifier) = &options.local_identifier {
            bstack_options.insert(
                "localIdentifier".to_string(),
                Value::String(local_identifier.clone()),
            );
        }

        let mut always_match = serde_json::Map::new();
        always_match.insert(
            "browserName".to_string(),
            Value::String(environment.browser.clone()),
        );
        if let Some(browser_version) = &environment.browser_version {
            always_match.insert(
                "browserVersion".to_string(),
                Value::String(browser_version.clone()),
            );
        }
        always_match.insert("bstack:options".to_string(), Value::Object(bstack_options));

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

    /// Configures WebDriver script and page-load timeouts.
    pub fn set_timeouts(
        &self,
        session_id: &str,
        script_timeout: Duration,
        page_load_timeout: Duration,
    ) -> Result<()> {
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

    /// Navigates the remote browser to the benchmark page.
    pub fn navigate(&self, session_id: &str, url: &str) -> Result<()> {
        let parsed = reqwest::Url::parse(url).context("parsing web benchmark URL")?;
        if !matches!(parsed.scheme(), "http" | "https") {
            bail!("web benchmark URL must use http or https");
        }
        let path = session_path(session_id, "url")?;
        let _: Value = self.send_json(
            self.authenticated(self.http.post(self.endpoint(&path))),
            Some(&json!({ "url": parsed.as_str() })),
            "navigating BrowserStack Automate session",
        )?;
        Ok(())
    }

    /// Executes synchronous JavaScript in the current page.
    pub fn execute_script(&self, session_id: &str, script: &str, args: &[Value]) -> Result<Value> {
        self.execute(session_id, "execute/sync", script, args)
    }

    /// Executes asynchronous JavaScript in the current page.
    pub fn execute_async_script(
        &self,
        session_id: &str,
        script: &str,
        args: &[Value],
    ) -> Result<Value> {
        self.execute(session_id, "execute/async", script, args)
    }

    /// Waits until the generated mobench browser harness is callable.
    pub fn wait_until_ready(&self, session_id: &str) -> Result<()> {
        self.wait_until_ready_with(session_id, DEFAULT_READY_TIMEOUT, DEFAULT_READY_POLL)
    }

    fn wait_until_ready_with(
        &self,
        session_id: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<()> {
        let started = Instant::now();
        loop {
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
            std::thread::sleep(poll_interval);
        }
    }

    /// Invokes `window.mobench.run(spec)` and returns its structured JSON value.
    pub fn run_benchmark(&self, session_id: &str, spec: &Value) -> Result<Value> {
        let value = self.execute_async_script(
            session_id,
            r#"
const spec = arguments[0];
const done = arguments[arguments.length - 1];
Promise.resolve()
  .then(() => window.mobench.run(spec))
  .then((result) => done({ ok: true, result }))
  .catch((error) => done({
    ok: false,
    error: error && error.message ? String(error.message) : String(error)
  }));
"#,
            std::slice::from_ref(spec),
        )?;

        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            let message = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("web benchmark failed without an error message");
            bail!("BrowserStack web benchmark failed: {message}");
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("BrowserStack web benchmark returned no result"))
    }

    /// Runs a complete mobench browser session and always attempts cleanup.
    ///
    /// The benchmark URL must already be reachable by the remote browser. For
    /// loopback or private URLs, start BrowserStack Local separately and set
    /// `request.options.local` plus its matching `local_identifier`.
    pub fn run_mobench_session(&self, request: &AutomateRunRequest) -> Result<RunnerReport> {
        let session = self.create_session(&request.environment, &request.options)?;
        let run_result = (|| {
            self.set_timeouts(
                &session.session_id,
                request.script_timeout,
                request.page_load_timeout,
            )?;
            self.navigate(&session.session_id, &request.url)?;
            self.wait_until_ready(&session.session_id)?;
            let spec =
                serde_json::to_value(&request.spec).context("serializing browser BenchSpec")?;
            let value = self.run_benchmark(&session.session_id, &spec)?;
            serde_json::from_value::<RunnerReport>(value).context("decoding browser RunnerReport")
        })();

        let passed = run_result.is_ok();
        let status_result = self.set_session_status(
            &session.session_id,
            passed,
            if passed {
                "mobench benchmark completed"
            } else {
                "mobench benchmark failed"
            },
        );
        let quit_result = self.quit(&session.session_id);

        match run_result {
            Ok(report) => {
                status_result.context("marking successful BrowserStack session")?;
                quit_result.context("closing successful BrowserStack session")?;
                Ok(report)
            }
            Err(run_error) => {
                if let Err(status_error) = status_result {
                    eprintln!(
                        "Warning: failed to mark BrowserStack session as failed: {status_error}"
                    );
                }
                if let Err(quit_error) = quit_result {
                    return Err(run_error.context(format!(
                        "also failed to close BrowserStack session: {quit_error}"
                    )));
                }
                Err(run_error)
            }
        }
    }

    /// Marks the BrowserStack session as passed or failed.
    pub fn set_session_status(&self, session_id: &str, passed: bool, reason: &str) -> Result<()> {
        validate_capability("session status reason", reason)?;
        let status = if passed { "passed" } else { "failed" };
        let executor = json!({
            "action": "setSessionStatus",
            "arguments": {
                "status": status,
                "reason": reason
            }
        });
        let script = format!("browserstack_executor: {executor}");
        let _: Value = self.execute_script(session_id, &script, &[])?;
        Ok(())
    }

    /// Ends the remote browser session.
    pub fn quit(&self, session_id: &str) -> Result<()> {
        let path = session_path(session_id, "")?;
        let response = self
            .authenticated(self.http.delete(self.endpoint(&path)))
            .send()
            .context("quitting BrowserStack Automate session")?;
        parse_webdriver_response::<Value>(response, "quitting BrowserStack Automate session")?;
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
        let response = request.send().with_context(|| context.to_string())?;
        parse_webdriver_response(response, context)
    }

    fn authenticated(&self, request: RequestBuilder) -> RequestBuilder {
        request.basic_auth(&self.auth.username, Some(&self.auth.access_key))
    }

    fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.webdriver_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
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
    let capabilities = value
        .get("capabilities")
        .cloned()
        .unwrap_or_else(|| json!({}));
    Ok(AutomateSession {
        session_id: session_id.to_string(),
        capabilities,
    })
}

fn parse_webdriver_response<T: DeserializeOwned>(
    mut response: Response,
    context: &str,
) -> Result<T> {
    let status = response.status();
    let content_length = response.content_length();
    if content_length.is_some_and(|bytes| bytes > MAX_WEBDRIVER_RESPONSE_BYTES as u64) {
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
        let message = webdriver_error_message(webdriver_value);
        bail!("{context}: BrowserStack returned HTTP {status}: {message}");
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
    let raw = value
        .get("message")
        .or_else(|| value.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("unknown WebDriver error");
    let mut message = raw
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(1024)
        .collect::<String>();
    if raw.chars().count() > 1024 {
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
        || session_id.len() > 256
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("BrowserStack returned an invalid WebDriver session id");
    }
    Ok(())
}

fn validate_capability(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        bail!("{label} must be a non-empty printable value of at most 256 characters");
    }
    Ok(())
}

fn validate_optional_capability(label: &str, value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        validate_capability(label, value)?;
    }
    Ok(())
}

fn duration_millis(duration: Duration) -> Result<u64> {
    u64::try_from(duration.as_millis()).context("WebDriver timeout is too large")
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
    }

    fn client(webdriver_url: impl Into<String>) -> BrowserStackAutomateClient {
        BrowserStackAutomateClient::new(BrowserStackAuth {
            username: "user".to_string(),
            access_key: "key".to_string(),
        })
        .unwrap()
        .with_webdriver_url(webdriver_url)
    }

    #[test]
    fn debug_output_redacts_browserstack_access_key() {
        let auth = BrowserStackAuth {
            username: "user".to_string(),
            access_key: "super-secret-key".to_string(),
        };

        let debug = format!("{auth:?}");

        assert!(debug.contains("user"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-key"));
    }

    fn spawn_webdriver_server(
        responses: Vec<Value>,
    ) -> (
        String,
        Arc<Mutex<Vec<RecordedRequest>>>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind WebDriver test server");
        let addr = listener.local_addr().expect("read WebDriver test address");
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let recorded_for_thread = Arc::clone(&recorded);
        let mut responses = VecDeque::from(responses);
        let expected_requests = responses.len();

        let handle = thread::spawn(move || {
            for _ in 0..expected_requests {
                let (mut stream, _) = listener.accept().expect("accept WebDriver request");
                let mut bytes = Vec::new();
                let mut header = [0_u8; 4096];
                let read = stream.read(&mut header).expect("read WebDriver request");
                bytes.extend_from_slice(&header[..read]);
                let request = String::from_utf8_lossy(&bytes);
                let first_line = request.lines().next().expect("request line");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                let content_length = request
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length: ")
                            .or_else(|| line.strip_prefix("content-length: "))
                    })
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let body_start = bytes
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|index| index + 4)
                    .unwrap_or(bytes.len());
                while bytes.len().saturating_sub(body_start) < content_length {
                    let mut chunk = [0_u8; 4096];
                    let read = stream.read(&mut chunk).expect("read WebDriver body");
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let body = if content_length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(
                        &bytes[body_start..body_start.saturating_add(content_length)],
                    )
                    .expect("parse request body")
                };
                recorded_for_thread
                    .lock()
                    .unwrap()
                    .push(RecordedRequest { method, path, body });

                let body = responses.pop_front().expect("queued response").to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write WebDriver response");
            }
        });

        (format!("http://{addr}/wd/hub"), recorded, handle)
    }

    #[test]
    fn creates_w3c_desktop_session() {
        let (url, recorded, handle) = spawn_webdriver_server(vec![json!({
            "value": {
                "sessionId": "session-123",
                "capabilities": { "browserName": "chrome" }
            }
        })]);
        let environment =
            BrowserEnvironment::desktop("chrome", Some("latest".into()), "OS X", "Sequoia");
        let options = AutomateSessionOptions {
            project_name: Some("mobench".into()),
            build_name: Some("web-main".into()),
            session_name: Some("sample::bench".into()),
            ..Default::default()
        };

        let session = client(url)
            .create_session(&environment, &options)
            .expect("create session");
        handle.join().unwrap();

        assert_eq!(session.session_id, "session-123");
        let requests = recorded.lock().unwrap();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/wd/hub/session");
        let always_match = &requests[0].body["capabilities"]["alwaysMatch"];
        assert_eq!(always_match["browserName"], "chrome");
        assert_eq!(always_match["browserVersion"], "latest");
        assert_eq!(always_match["bstack:options"]["os"], "OS X");
        assert_eq!(always_match["bstack:options"]["projectName"], "mobench");
    }

    #[test]
    fn creates_real_mobile_session_with_local_tunnel() {
        let (url, recorded, handle) = spawn_webdriver_server(vec![json!({
            "value": { "sessionId": "mobile_123", "capabilities": {} }
        })]);
        let options = AutomateSessionOptions {
            local: true,
            local_identifier: Some("mobench-run-1".into()),
            ..Default::default()
        };

        client(url)
            .create_session(
                &BrowserEnvironment::mobile("chrome", "Google Pixel 9", "16.0"),
                &options,
            )
            .expect("create mobile session");
        handle.join().unwrap();

        let requests = recorded.lock().unwrap();
        let bstack = &requests[0].body["capabilities"]["alwaysMatch"]["bstack:options"];
        assert_eq!(bstack["deviceName"], "Google Pixel 9");
        assert_eq!(bstack["realMobile"], "true");
        assert_eq!(bstack["local"], "true");
        assert_eq!(bstack["localIdentifier"], "mobench-run-1");
    }

    #[test]
    fn navigates_waits_and_returns_structured_benchmark_result() {
        let expected = json!({
            "function": "sample::bench",
            "samples": [{ "duration_ns": 42 }]
        });
        let (url, recorded, handle) = spawn_webdriver_server(vec![
            json!({ "value": null }),
            json!({ "value": true }),
            json!({ "value": { "ok": true, "result": expected } }),
        ]);
        let client = client(url);

        client
            .navigate("session-123", "http://localhost:8080/")
            .expect("navigate");
        client
            .wait_until_ready_with("session-123", Duration::from_secs(1), Duration::ZERO)
            .expect("ready");
        let result = client
            .run_benchmark("session-123", &json!({ "name": "sample::bench" }))
            .expect("run benchmark");
        handle.join().unwrap();

        assert_eq!(result, expected);
        let requests = recorded.lock().unwrap();
        assert_eq!(requests[0].path, "/wd/hub/session/session-123/url");
        assert_eq!(requests[1].path, "/wd/hub/session/session-123/execute/sync");
        assert_eq!(
            requests[2].path,
            "/wd/hub/session/session-123/execute/async"
        );
    }

    #[test]
    fn configures_timeouts_marks_status_and_quits() {
        let (url, recorded, handle) = spawn_webdriver_server(vec![
            json!({ "value": null }),
            json!({ "value": true }),
            json!({ "value": null }),
            json!({ "value": null }),
        ]);
        let client = client(url);

        client
            .set_timeouts(
                "session-123",
                Duration::from_secs(90),
                Duration::from_secs(30),
            )
            .expect("set timeouts");
        client
            .wait_until_ready("session-123")
            .expect("wait for harness");
        client
            .set_session_status("session-123", true, "benchmark completed")
            .expect("set session status");
        client.quit("session-123").expect("quit session");
        handle.join().unwrap();

        let requests = recorded.lock().unwrap();
        assert_eq!(requests[0].path, "/wd/hub/session/session-123/timeouts");
        assert_eq!(requests[0].body["script"], 90_000);
        assert_eq!(requests[0].body["pageLoad"], 30_000);
        assert_eq!(
            requests[2].body["script"],
            "browserstack_executor: {\"action\":\"setSessionStatus\",\"arguments\":{\"reason\":\"benchmark completed\",\"status\":\"passed\"}}"
        );
        assert_eq!(requests[3].method, "DELETE");
        assert_eq!(requests[3].path, "/wd/hub/session/session-123");
    }

    #[test]
    fn complete_session_returns_typed_report_and_cleans_up() {
        let report = json!({
            "spec": {
                "name": "sample::bench",
                "iterations": 1,
                "warmup": 0
            },
            "samples": [{
                "iteration": 0,
                "duration_ns": 42
            }],
            "summary": {
                "mean_ns": 42.0,
                "median_ns": 42.0,
                "min_ns": 42,
                "max_ns": 42,
                "std_dev_ns": 0.0,
                "p95_ns": 42.0
            }
        });
        let (url, recorded, handle) = spawn_webdriver_server(vec![
            json!({ "value": { "sessionId": "session-123", "capabilities": {} } }),
            json!({ "value": null }),
            json!({ "value": null }),
            json!({ "value": true }),
            json!({ "value": { "ok": true, "result": report } }),
            json!({ "value": null }),
            json!({ "value": null }),
        ]);
        let client = client(url);
        let mut request = AutomateRunRequest::new(
            BrowserEnvironment::desktop("chrome", None, "OS X", "Sequoia"),
            "https://bench.example.test/",
            BenchSpec {
                name: "sample::bench".into(),
                iterations: 1,
                warmup: 0,
            },
        );
        request.script_timeout = Duration::from_secs(1);
        request.page_load_timeout = Duration::from_secs(1);

        let result = client
            .run_mobench_session(&request)
            .expect("complete BrowserStack session");
        handle.join().unwrap();

        assert_eq!(result.spec.name, "sample::bench");
        assert_eq!(result.samples.len(), 1);
        let requests = recorded.lock().unwrap();
        assert_eq!(requests.len(), 7);
        assert_eq!(requests[6].method, "DELETE");
        assert_eq!(requests[6].path, "/wd/hub/session/session-123");
    }

    #[test]
    fn complete_session_marks_failure_and_still_quits() {
        let (url, recorded, handle) = spawn_webdriver_server(vec![
            json!({ "value": { "sessionId": "session-123", "capabilities": {} } }),
            json!({ "value": null }),
            json!({ "value": null }),
            json!({ "value": true }),
            json!({ "value": { "ok": false, "error": "benchmark exploded" } }),
            json!({ "value": null }),
            json!({ "value": null }),
        ]);
        let client = client(url);
        let mut request = AutomateRunRequest::new(
            BrowserEnvironment::desktop("chrome", None, "OS X", "Sequoia"),
            "https://bench.example.test/",
            BenchSpec {
                name: "sample::bench".into(),
                iterations: 1,
                warmup: 0,
            },
        );
        request.script_timeout = Duration::from_secs(1);
        request.page_load_timeout = Duration::from_secs(1);

        let error = client
            .run_mobench_session(&request)
            .expect_err("benchmark failure must propagate");
        handle.join().unwrap();

        assert!(error.to_string().contains("benchmark exploded"));
        let requests = recorded.lock().unwrap();
        assert!(
            requests[5].body["script"]
                .as_str()
                .unwrap()
                .contains(r#""status":"failed""#)
        );
        assert_eq!(requests[6].method, "DELETE");
    }

    #[test]
    fn rejects_unsafe_session_ids_before_request() {
        let client = client("http://127.0.0.1:9/wd/hub");
        let error = client
            .navigate("../sessions", "https://example.com")
            .expect_err("unsafe session id must fail");
        assert!(error.to_string().contains("invalid WebDriver session id"));
    }

    #[test]
    fn requires_local_mode_for_local_identifier() {
        let error = AutomateSessionOptions {
            local_identifier: Some("run-1".into()),
            ..Default::default()
        }
        .validate()
        .expect_err("identifier without local must fail");
        assert!(error.to_string().contains("requires local=true"));
    }

    #[test]
    fn unwraps_webdriver_errors() {
        let response = json!({
            "value": {
                "error": "javascript error",
                "message": "window.mobench is undefined"
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);
            let body = response.to_string();
            let reply = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(reply.as_bytes()).unwrap();
        });

        let error = client(format!("http://{addr}/wd/hub"))
            .execute_script("session-123", "return true;", &[])
            .expect_err("WebDriver error must fail");
        handle.join().unwrap();
        assert!(error.to_string().contains("window.mobench is undefined"));
    }

    #[test]
    fn rejects_oversized_webdriver_responses_before_reading_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_WEBDRIVER_RESPONSE_BYTES + 1
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let error = client(format!("http://{addr}/wd/hub"))
            .execute_script("session-123", "return true;", &[])
            .expect_err("oversized response must fail");
        handle.join().unwrap();
        assert!(error.to_string().contains("exceeded the size limit"));
    }
}
