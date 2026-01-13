use anyhow::{Context, Result, anyhow, bail};
use bench_runner::{BenchSpec, run_closure};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use browserstack::{BrowserStackAuth, BrowserStackClient};

mod browserstack;

/// CLI orchestrator for building, packaging, and executing Rust benchmarks on mobile.
#[derive(Parser, Debug)]
#[command(name = "mobench", author, version, about = "Mobile Rust benchmarking orchestrator", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a benchmark against a target platform (mobile integration stub for now).
    Run {
        #[arg(long, value_enum)]
        target: MobileTarget,
        #[arg(long, help = "Fully-qualified Rust function to benchmark")]
        function: String,
        #[arg(long, default_value_t = 100)]
        iterations: u32,
        #[arg(long, default_value_t = 10)]
        warmup: u32,
        #[arg(long, help = "Device identifiers or labels (BrowserStack devices)")]
        devices: Vec<String>,
        #[arg(long, help = "Optional path to config file")]
        config: Option<PathBuf>,
        #[arg(long, help = "Optional output path for JSON report")]
        output: Option<PathBuf>,
        #[arg(long, help = "Skip mobile builds and only run the host harness")]
        local_only: bool,
        #[arg(
            long,
            help = "Path to iOS app bundle (.ipa or zipped .app) for BrowserStack XCUITest"
        )]
        ios_app: Option<PathBuf>,
        #[arg(long, help = "Path to iOS XCUITest test suite package (.zip or .ipa)")]
        ios_test_suite: Option<PathBuf>,
        #[arg(long, help = "Fetch BrowserStack artifacts after the run completes")]
        fetch: bool,
        #[arg(long, default_value = "target/browserstack")]
        fetch_output_dir: PathBuf,
        #[arg(long, default_value_t = 10)]
        fetch_poll_interval_secs: u64,
        #[arg(long, default_value_t = 1800)]
        fetch_timeout_secs: u64,
    },
    /// Run a local demo against bundled sample functions to validate the harness.
    Demo {
        #[arg(long, default_value_t = 50)]
        iterations: u32,
        #[arg(long, default_value_t = 5)]
        warmup: u32,
    },
    /// Scaffold a base config file for the CLI.
    Init {
        #[arg(long, default_value = "bench-config.toml")]
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = MobileTarget::Android)]
        target: MobileTarget,
    },
    /// Generate a sample device matrix file.
    Plan {
        #[arg(long, default_value = "device-matrix.yaml")]
        output: PathBuf,
    },
    /// Fetch BrowserStack build artifacts (logs, session JSON) for CI.
    Fetch {
        #[arg(long, value_enum)]
        target: MobileTarget,
        #[arg(long)]
        build_id: String,
        #[arg(long, default_value = "target/browserstack")]
        output_dir: PathBuf,
        #[arg(long, default_value_t = true)]
        wait: bool,
        #[arg(long, default_value_t = 10)]
        poll_interval_secs: u64,
        #[arg(long, default_value_t = 1800)]
        timeout_secs: u64,
    },
    /// Initialize a new benchmark project with SDK (Phase 1 MVP).
    InitSdk {
        #[arg(long, value_enum)]
        target: SdkTarget,
        #[arg(long, default_value = "bench-project")]
        project_name: String,
        #[arg(long, default_value = ".")]
        output_dir: PathBuf,
        #[arg(long, help = "Generate example benchmarks")]
        examples: bool,
    },
    /// Build mobile artifacts (Phase 1 MVP).
    Build {
        #[arg(long, value_enum)]
        target: SdkTarget,
        #[arg(long, help = "Build in release mode")]
        release: bool,
    },
    /// Package iOS app as IPA for distribution or testing.
    PackageIpa {
        #[arg(long, default_value = "BenchRunner", help = "Xcode scheme to build")]
        scheme: String,
        #[arg(long, value_enum, default_value = "adhoc", help = "Signing method")]
        method: IosSigningMethodArg,
    },
    /// List all discovered benchmark functions (Phase 1 MVP).
    List,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MobileTarget {
    Android,
    Ios,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lowercase")]
enum SdkTarget {
    Android,
    Ios,
    Both,
}

impl From<SdkTarget> for mobench_sdk::Target {
    fn from(target: SdkTarget) -> Self {
        match target {
            SdkTarget::Android => mobench_sdk::Target::Android,
            SdkTarget::Ios => mobench_sdk::Target::Ios,
            SdkTarget::Both => mobench_sdk::Target::Both,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lowercase")]
enum IosSigningMethodArg {
    /// Ad-hoc signing (no Apple ID needed, works for BrowserStack)
    Adhoc,
    /// Development signing (requires Apple Developer account)
    Development,
}

impl From<IosSigningMethodArg> for mobench_sdk::builders::SigningMethod {
    fn from(arg: IosSigningMethodArg) -> Self {
        match arg {
            IosSigningMethodArg::Adhoc => mobench_sdk::builders::SigningMethod::AdHoc,
            IosSigningMethodArg::Development => mobench_sdk::builders::SigningMethod::Development,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BrowserStackConfig {
    app_automate_username: String,
    app_automate_access_key: String,
    project: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct IosXcuitestArtifacts {
    app: PathBuf,
    test_suite: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchConfig {
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    device_matrix: PathBuf,
    browserstack: BrowserStackConfig,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    ios_xcuitest: Option<IosXcuitestArtifacts>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct DeviceEntry {
    name: String,
    os: String,
    os_version: String,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeviceMatrix {
    devices: Vec<DeviceEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RunSpec {
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    devices: Vec<String>,
    #[serde(skip_serializing, skip_deserializing, default)]
    browserstack: Option<BrowserStackConfig>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    ios_xcuitest: Option<IosXcuitestArtifacts>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "platform", rename_all = "lowercase")]
enum MobileArtifacts {
    Android {
        apk: PathBuf,
    },
    Ios {
        xcframework: PathBuf,
        header: PathBuf,
        #[serde(skip_serializing_if = "Option::is_none")]
        app: Option<PathBuf>,
        #[serde(skip_serializing_if = "Option::is_none")]
        test_suite: Option<PathBuf>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct RunSummary {
    spec: RunSpec,
    artifacts: Option<MobileArtifacts>,
    local_report: Value,
    remote_run: Option<RemoteRun>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "platform", rename_all = "lowercase")]
enum RemoteRun {
    Android {
        app_url: String,
        build_id: String,
    },
    Ios {
        app_url: String,
        test_suite_url: String,
        build_id: String,
    },
}

fn main() -> Result<()> {
    load_dotenv();
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            target,
            function,
            iterations,
            warmup,
            devices,
            config,
            output,
            local_only,
            ios_app,
            ios_test_suite,
            fetch,
            fetch_output_dir,
            fetch_poll_interval_secs,
            fetch_timeout_secs,
        } => {
            let spec = resolve_run_spec(
                target,
                function,
                iterations,
                warmup,
                devices,
                config.as_deref(),
                ios_app,
                ios_test_suite,
            )?;
            println!(
                "Preparing benchmark run for {:?}: {} (iterations={}, warmup={})",
                spec.target, spec.function, spec.iterations, spec.warmup
            );
            persist_mobile_spec(&spec)?;
            if !spec.devices.is_empty() {
                println!("Devices: {}", spec.devices.join(", "));
            }
            if let Some(path) = &output {
                println!("JSON summary will be written to {:?}", path);
            }

            let local_report = run_local_smoke(&spec)?;
            let mut remote_run = None;
            let artifacts = if local_only {
                println!("Skipping mobile build: --local-only set");
                None
            } else {
                match spec.target {
                    MobileTarget::Android => {
                        let ndk = std::env::var("ANDROID_NDK_HOME")
                            .context("ANDROID_NDK_HOME must be set for Android builds")?;
                        let apk = run_android_build(&ndk)?;
                        println!("Built Android APK at {:?}", apk);
                        if spec.devices.is_empty() {
                            println!("Skipping BrowserStack upload/run: no devices provided");
                            Some(MobileArtifacts::Android { apk })
                        } else {
                            let run = trigger_browserstack_espresso(&spec, &apk)?;
                            remote_run = Some(run);
                            Some(MobileArtifacts::Android { apk })
                        }
                    }
                    MobileTarget::Ios => {
                        let (xcframework, header) = run_ios_build()?;
                        println!("Built iOS xcframework at {:?}", xcframework);
                        let ios_xcuitest = spec.ios_xcuitest.clone();

                        if spec.devices.is_empty() {
                            println!("Skipping BrowserStack upload/run: no devices provided");
                        } else {
                            let xcui = spec.ios_xcuitest.as_ref().context(
                                "iOS XCUITest artifacts required when targeting BrowserStack devices; provide --ios-app and --ios-test-suite or set ios_xcuitest in the config",
                            )?;
                            let run = trigger_browserstack_xcuitest(&spec, xcui)?;
                            remote_run = Some(run);
                        }

                        Some(MobileArtifacts::Ios {
                            xcframework,
                            header,
                            app: ios_xcuitest.as_ref().map(|a| a.app.clone()),
                            test_suite: ios_xcuitest.map(|a| a.test_suite),
                        })
                    }
                }
            };

            let summary = RunSummary {
                spec,
                artifacts,
                local_report,
                remote_run,
            };
            write_summary(&summary, output.as_deref())?;

            if fetch {
                if let Some(remote) = &summary.remote_run {
                    let build_id = match remote {
                        RemoteRun::Android { build_id, .. } => build_id,
                        RemoteRun::Ios { build_id, .. } => build_id,
                    };
                    let creds =
                        resolve_browserstack_credentials(summary.spec.browserstack.as_ref())?;
                    let client = BrowserStackClient::new(
                        BrowserStackAuth {
                            username: creds.username,
                            access_key: creds.access_key,
                        },
                        creds.project,
                    )?;
                    let output_root = fetch_output_dir.join(build_id);
                    fetch_browserstack_artifacts(
                        &client,
                        summary.spec.target,
                        build_id,
                        &output_root,
                        true,
                        fetch_poll_interval_secs,
                        fetch_timeout_secs,
                    )?;
                } else {
                    println!("No BrowserStack run to fetch (devices not provided?)");
                }
            }
        }
        Command::Demo { iterations, warmup } => {
            let spec = BenchSpec::new("sample_fns::fibonacci", iterations, warmup)?;
            let report = run_closure(spec, || {
                // This is the shape of the closure that will be invoked on-device;
                // for now we reuse it locally.
                let _ = sample_fns::fibonacci(24);
                Ok(())
            })?;

            let json = serde_json::to_string_pretty(&report)?;
            println!("{json}");
        }
        Command::Init { output, target } => {
            write_config_template(&output, target)?;
            println!("Wrote starter config to {:?}", output);
        }
        Command::Plan { output } => {
            write_device_matrix_template(&output)?;
            println!("Wrote sample device matrix to {:?}", output);
        }
        Command::Fetch {
            target,
            build_id,
            output_dir,
            wait,
            poll_interval_secs,
            timeout_secs,
        } => {
            let creds = resolve_browserstack_credentials(None)?;
            let client = BrowserStackClient::new(
                BrowserStackAuth {
                    username: creds.username,
                    access_key: creds.access_key,
                },
                creds.project,
            )?;
            let output_root = output_dir.join(&build_id);
            fetch_browserstack_artifacts(
                &client,
                target,
                &build_id,
                &output_root,
                wait,
                poll_interval_secs,
                timeout_secs,
            )?;
        }
        Command::InitSdk {
            target,
            project_name,
            output_dir,
            examples,
        } => {
            cmd_init_sdk(target, project_name, output_dir, examples)?;
        }
        Command::Build { target, release } => {
            cmd_build(target, release)?;
        }
        Command::PackageIpa { scheme, method } => {
            cmd_package_ipa(&scheme, method)?;
        }
        Command::List => {
            cmd_list()?;
        }
    }

    Ok(())
}

fn write_config_template(path: &Path, target: MobileTarget) -> Result<()> {
    ensure_can_write(path)?;

    let ios_xcuitest = if target == MobileTarget::Ios {
        Some(IosXcuitestArtifacts {
            app: PathBuf::from("target/ios/BenchRunner.ipa"),
            test_suite: PathBuf::from("target/ios/BenchRunnerUITests.zip"),
        })
    } else {
        None
    };

    let cfg = BenchConfig {
        target,
        function: "sample_fns::fibonacci".into(),
        iterations: 100,
        warmup: 10,
        device_matrix: PathBuf::from("device-matrix.yaml"),
        browserstack: BrowserStackConfig {
            app_automate_username: "${BROWSERSTACK_USERNAME}".into(),
            app_automate_access_key: "${BROWSERSTACK_ACCESS_KEY}".into(),
            project: Some("mobile-bench-rs".into()),
        },
        ios_xcuitest,
    };

    let contents = toml::to_string_pretty(&cfg)?;
    write_file(path, contents.as_bytes())
}

fn write_device_matrix_template(path: &Path) -> Result<()> {
    ensure_can_write(path)?;

    let matrix = DeviceMatrix {
        devices: vec![
            DeviceEntry {
                name: "Pixel 7".into(),
                os: "android".into(),
                os_version: "13.0".into(),
                tags: Some(vec!["default".into(), "pixel".into()]),
            },
            DeviceEntry {
                name: "iPhone 14".into(),
                os: "ios".into(),
                os_version: "16".into(),
                tags: Some(vec!["default".into(), "iphone".into()]),
            },
        ],
    };

    let contents = serde_yaml::to_string(&matrix)?;
    write_file(path, contents.as_bytes())
}

fn fetch_browserstack_artifacts(
    client: &BrowserStackClient,
    target: MobileTarget,
    build_id: &str,
    output_root: &Path,
    wait: bool,
    poll_interval_secs: u64,
    timeout_secs: u64,
) -> Result<()> {
    fs::create_dir_all(output_root)
        .with_context(|| format!("creating output dir {:?}", output_root))?;

    let base = browserstack_base_path(target);
    let build_path = format!("{base}/builds/{build_id}");
    let sessions_path = format!("{base}/builds/{build_id}/sessions");

    if wait {
        wait_for_build(client, &build_path, poll_interval_secs, timeout_secs)?;
    }

    let build_json = client.get_json(&build_path)?;
    write_json(output_root.join("build.json"), &build_json)?;

    let sessions_json = match client.get_json(&sessions_path) {
        Ok(value) => {
            write_json(output_root.join("sessions.json"), &value)?;
            Some(value)
        }
        Err(err) => {
            let msg = shorten_html_error(&err.to_string());
            println!("Sessions endpoint unavailable; falling back to build.json: {msg}");
            None
        }
    };

    let session_ids = extract_session_ids(sessions_json.as_ref().unwrap_or(&build_json));
    if session_ids.is_empty() {
        println!("No sessions found for build {}", build_id);
        return Ok(());
    }

    for session_id in session_ids {
        let session_path = format!("{base}/builds/{build_id}/sessions/{session_id}");
        let session_json = client.get_json(&session_path)?;
        let session_dir = output_root.join(format!("session-{}", session_id));
        fs::create_dir_all(&session_dir)
            .with_context(|| format!("creating session dir {:?}", session_dir))?;
        write_json(session_dir.join("session.json"), &session_json)?;

        let mut bench_report: Option<Value> = None;
        for (key, url) in extract_url_fields(&session_json) {
            let file_name = filename_for_url(&key, &url);
            let dest = session_dir.join(file_name);
            if let Err(err) = client.download_url(&url, &dest) {
                println!("Skipping download for {key}: {err}");
                continue;
            }
            if (key.contains("device_log")
                || key.contains("instrumentation_log")
                || key.contains("app_log"))
                && let Ok(contents) = fs::read_to_string(&dest)
                && let Some(parsed) = extract_bench_json(&contents)
            {
                bench_report = Some(parsed);
            }
        }

        if let Some(report) = bench_report {
            write_json(session_dir.join("bench-report.json"), &report)?;
        }
    }

    println!("Fetched BrowserStack artifacts to {:?}", output_root);
    Ok(())
}

fn browserstack_base_path(target: MobileTarget) -> &'static str {
    match target {
        MobileTarget::Android => "app-automate/espresso/v2",
        MobileTarget::Ios => "app-automate/xcuitest/v2",
    }
}

fn wait_for_build(
    client: &BrowserStackClient,
    build_path: &str,
    poll_interval_secs: u64,
    timeout_secs: u64,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        let build_json = client.get_json(build_path)?;
        if let Some(status) = build_json
            .get("status")
            .and_then(|val| val.as_str())
            .map(|val| val.to_lowercase())
        {
            if status == "failed" || status == "error" {
                println!("Build status: {status}");
                return Ok(());
            }
            if status == "done" || status == "passed" || status == "completed" {
                println!("Build status: {status}");
                return Ok(());
            }
            println!("Build status: {status} (waiting)");
        } else {
            println!("Build status missing; continuing without wait");
            return Ok(());
        }

        if Instant::now() >= deadline {
            println!("Timed out waiting for build status");
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(poll_interval_secs));
    }
}

fn extract_session_ids(value: &Value) -> Vec<String> {
    let sessions = value
        .get("sessions")
        .and_then(|val| val.as_array())
        .or_else(|| value.as_array());
    let mut ids = Vec::new();
    if let Some(entries) = sessions {
        for entry in entries {
            let id = entry
                .get("id")
                .or_else(|| entry.get("session_id"))
                .or_else(|| entry.get("sessionId"))
                .and_then(|val| val.as_str());
            if let Some(id) = id {
                ids.push(id.to_string());
            }
        }
    }
    if ids.is_empty()
        && let Some(devices) = value.get("devices").and_then(|val| val.as_array())
    {
        for device in devices {
            if let Some(sessions) = device.get("sessions").and_then(|val| val.as_array()) {
                for entry in sessions {
                    if let Some(id) = entry.get("id").and_then(|val| val.as_str()) {
                        ids.push(id.to_string());
                    }
                }
            }
        }
    }
    ids
}

fn extract_url_fields(value: &Value) -> Vec<(String, String)> {
    let mut urls = Vec::new();
    extract_url_fields_recursive(value, "", &mut urls);
    urls
}

fn extract_url_fields_recursive(value: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                let next = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                if let Value::String(url) = val
                    && (url.starts_with("http") || url.starts_with("bs://"))
                {
                    out.push((next.clone(), url.clone()));
                }
                extract_url_fields_recursive(val, &next, out);
            }
        }
        Value::Array(items) => {
            for (idx, val) in items.iter().enumerate() {
                let next = format!("{}[{}]", prefix, idx);
                extract_url_fields_recursive(val, &next, out);
            }
        }
        _ => {}
    }
}

fn filename_for_url(key: &str, url: &str) -> String {
    let stripped = url.split('?').next().unwrap_or(url);
    let ext = Path::new(stripped)
        .extension()
        .and_then(|val| val.to_str())
        .unwrap_or("log");
    let mut safe = String::with_capacity(key.len());
    for ch in key.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            safe.push(ch);
        } else {
            safe.push('_');
        }
    }
    format!("{}.{}", safe, ext)
}

fn extract_bench_json(contents: &str) -> Option<Value> {
    let marker = "BENCH_JSON ";
    for line in contents.lines().rev() {
        if let Some(idx) = line.find(marker) {
            let json_part = &line[idx + marker.len()..];
            if let Ok(value) = serde_json::from_str::<Value>(json_part) {
                return Some(value);
            }
        }
    }
    None
}

fn write_json(path: PathBuf, value: &Value) -> Result<()> {
    let contents = serde_json::to_string_pretty(value)?;
    write_file(&path, contents.as_bytes())
}

fn shorten_html_error(message: &str) -> String {
    if message.contains("<!DOCTYPE html>") || message.contains("<html") {
        return "received HTML response (check BrowserStack API endpoint)".to_string();
    }
    message.to_string()
}

fn resolve_run_spec(
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    devices: Vec<String>,
    config: Option<&Path>,
    ios_app: Option<PathBuf>,
    ios_test_suite: Option<PathBuf>,
) -> Result<RunSpec> {
    if let Some(cfg_path) = config {
        let cfg = load_config(cfg_path)?;
        let matrix = load_device_matrix(&cfg.device_matrix)?;
        let device_names = matrix.devices.into_iter().map(|d| d.name).collect();
        return Ok(RunSpec {
            target: cfg.target,
            function: cfg.function,
            iterations: cfg.iterations,
            warmup: cfg.warmup,
            devices: device_names,
            browserstack: Some(cfg.browserstack),
            ios_xcuitest: cfg.ios_xcuitest,
        });
    }

    if function.trim().is_empty() {
        bail!("function must not be empty");
    }

    let ios_xcuitest = match (ios_app, ios_test_suite) {
        (Some(app), Some(test_suite)) => Some(IosXcuitestArtifacts { app, test_suite }),
        (None, None) => None,
        _ => bail!("both --ios-app and --ios-test-suite must be provided together"),
    };

    if target == MobileTarget::Ios && !devices.is_empty() && ios_xcuitest.is_none() {
        bail!(
            "iOS BrowserStack runs require --ios-app and --ios-test-suite or an ios_xcuitest config block"
        );
    }

    Ok(RunSpec {
        target,
        function,
        iterations,
        warmup,
        devices,
        browserstack: None,
        ios_xcuitest,
    })
}

fn load_config(path: &Path) -> Result<BenchConfig> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading config {:?}", path))?;
    toml::from_str(&contents).with_context(|| format!("parsing config {:?}", path))
}

fn load_device_matrix(path: &Path) -> Result<DeviceMatrix> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading device matrix {:?}", path))?;
    serde_yaml::from_str(&contents).with_context(|| format!("parsing device matrix {:?}", path))
}

fn run_ios_build() -> Result<(PathBuf, PathBuf)> {
    let root = repo_root()?;
    let crate_name =
        detect_bench_mobile_crate_name(&root).unwrap_or_else(|_| "bench-mobile".to_string());
    let builder = mobench_sdk::builders::IosBuilder::new(&root, crate_name).verbose(true);
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Ios,
        profile: mobench_sdk::BuildProfile::Debug,
        incremental: true,
    };
    let result = builder.build(&cfg)?;
    let header = root.join("target/ios/include").join(format!(
        "{}.h",
        result
            .app_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("module")
    ));
    Ok((result.app_path, header))
}

#[derive(Debug, Clone)]
struct ResolvedBrowserStack {
    username: String,
    access_key: String,
    project: Option<String>,
}

fn trigger_browserstack_espresso(spec: &RunSpec, apk: &Path) -> Result<RemoteRun> {
    let creds = resolve_browserstack_credentials(spec.browserstack.as_ref())?;
    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username.clone(),
            access_key: creds.access_key.clone(),
        },
        creds.project.clone(),
    )?;

    // Upload the app-under-test APK.
    let upload = client.upload_espresso_app(apk)?;

    // Upload the Espresso test-suite APK produced by Gradle.
    // We rely on the standard androidTest debug output path.
    let root = repo_root()?;
    let test_apk =
        root.join("android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk");
    let test_upload = client.upload_espresso_test_suite(&test_apk)?;

    // Schedule the Espresso build with both app and testSuite, as required by BrowserStack.
    let run = client.schedule_espresso_run(
        &spec.devices,
        &upload.app_url,
        &test_upload.test_suite_url,
    )?;
    println!(
        "Queued BrowserStack Espresso build {} for devices: {}",
        run.build_id,
        spec.devices.join(", ")
    );

    Ok(RemoteRun::Android {
        app_url: upload.app_url,
        build_id: run.build_id,
    })
}

fn trigger_browserstack_xcuitest(
    spec: &RunSpec,
    artifacts: &IosXcuitestArtifacts,
) -> Result<RemoteRun> {
    let creds = resolve_browserstack_credentials(spec.browserstack.as_ref())?;
    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username.clone(),
            access_key: creds.access_key.clone(),
        },
        creds.project.clone(),
    )?;

    if !artifacts.app.exists() {
        bail!(
            "iOS app artifact not found at {:?}; provide a .ipa or zipped .app",
            artifacts.app
        );
    }
    if !artifacts.test_suite.exists() {
        bail!(
            "iOS XCUITest test suite artifact not found at {:?}; provide the zipped test runner bundle",
            artifacts.test_suite
        );
    }

    let app_upload = client.upload_xcuitest_app(&artifacts.app)?;
    let test_upload = client.upload_xcuitest_test_suite(&artifacts.test_suite)?;
    let run = client.schedule_xcuitest_run(
        &spec.devices,
        &app_upload.app_url,
        &test_upload.test_suite_url,
    )?;
    println!(
        "Queued BrowserStack XCUITest build {} for devices: {}",
        run.build_id,
        spec.devices.join(", ")
    );

    Ok(RemoteRun::Ios {
        app_url: app_upload.app_url,
        test_suite_url: test_upload.test_suite_url,
        build_id: run.build_id,
    })
}

fn resolve_browserstack_credentials(
    config: Option<&BrowserStackConfig>,
) -> Result<ResolvedBrowserStack> {
    let mut username = None;
    let mut access_key = None;
    let mut project = None;

    if let Some(cfg) = config {
        username = Some(expand_env_var(&cfg.app_automate_username)?);
        access_key = Some(expand_env_var(&cfg.app_automate_access_key)?);
        project = cfg
            .project
            .as_ref()
            .map(|p| expand_env_var(p))
            .transpose()?;
    }

    if username.as_deref().map(str::is_empty).unwrap_or(true)
        && let Ok(val) = env::var("BROWSERSTACK_USERNAME")
        && !val.is_empty()
    {
        username = Some(val);
    }
    if access_key.as_deref().map(str::is_empty).unwrap_or(true)
        && let Ok(val) = env::var("BROWSERSTACK_ACCESS_KEY")
        && !val.is_empty()
    {
        access_key = Some(val);
    }
    if project.is_none()
        && let Ok(val) = env::var("BROWSERSTACK_PROJECT")
        && !val.is_empty()
    {
        project = Some(val);
    }

    let username = username.filter(|s| !s.is_empty()).ok_or_else(|| {
        anyhow!("BrowserStack username missing; set BROWSERSTACK_USERNAME or provide in config")
    })?;
    let access_key = access_key.filter(|s| !s.is_empty()).ok_or_else(|| {
        anyhow!("BrowserStack access key missing; set BROWSERSTACK_ACCESS_KEY or provide in config")
    })?;

    Ok(ResolvedBrowserStack {
        username,
        access_key,
        project,
    })
}

fn expand_env_var(raw: &str) -> Result<String> {
    if let Some(stripped) = raw.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        let val = env::var(stripped)
            .with_context(|| format!("resolving env var {stripped} for BrowserStack config"))?;
        return Ok(val);
    }
    Ok(raw.to_string())
}

fn run_local_smoke(spec: &RunSpec) -> Result<Value> {
    let bench_spec = mobench_sdk::BenchSpec {
        name: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
    };

    let report =
        mobench_sdk::run_benchmark(bench_spec).map_err(|e| anyhow!("benchmark failed: {:?}", e))?;

    serde_json::to_value(&report).context("serializing benchmark report")
}

fn persist_mobile_spec(spec: &RunSpec) -> Result<()> {
    let root = repo_root()?;
    let payload = json!({
        "function": spec.function,
        "iterations": spec.iterations,
        "warmup": spec.warmup,
    });
    let contents = serde_json::to_string_pretty(&payload)?;
    let targets = [
        root.join("target/mobile-spec/android/bench_spec.json"),
        root.join("target/mobile-spec/ios/bench_spec.json"),
    ];
    for path in targets {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {:?}", parent))?;
        }
        write_file(&path, contents.as_bytes())?;
    }
    Ok(())
}

fn write_summary(summary: &RunSummary, output: Option<&Path>) -> Result<()> {
    let json = serde_json::to_string_pretty(summary)?;
    if let Some(path) = output {
        write_file(path, json.as_bytes())?;
        println!("Wrote run summary to {:?}", path);
    } else {
        println!("{json}");
    }
    Ok(())
}

fn run_android_build(_ndk_home: &str) -> Result<PathBuf> {
    let root = repo_root()?;
    let crate_name =
        detect_bench_mobile_crate_name(&root).unwrap_or_else(|_| "bench-mobile".to_string());

    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Android,
        profile: mobench_sdk::BuildProfile::Debug,
        incremental: true,
    };
    let builder = mobench_sdk::builders::AndroidBuilder::new(&root, crate_name).verbose(true);
    let result = builder.build(&cfg)?;
    Ok(result.app_path)
}

fn load_dotenv() {
    if let Ok(root) = repo_root() {
        let path = root.join(".env.local");
        let _ = dotenvy::from_path(path);
    }
}

fn repo_root() -> Result<PathBuf> {
    // Prefer the build-time repo root but fall back to the current directory for installed binaries.
    let compiled = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    if let Ok(path) = compiled.canonicalize() {
        return Ok(path);
    }
    std::env::current_dir().context("resolving repo root from current directory")
}

fn ensure_can_write(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("refusing to overwrite existing file: {:?}", path);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory {:?}", parent))?;
    }
    Ok(())
}

fn write_file(path: &Path, contents: &[u8]) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("writing file {:?}", path))
}

/// Initialize a new benchmark project using mobench-sdk (Phase 1 MVP)
fn cmd_init_sdk(
    target: SdkTarget,
    project_name: String,
    output_dir: PathBuf,
    generate_examples: bool,
) -> Result<()> {
    println!("Initializing benchmark project with mobench-sdk...");
    println!("  Project name: {}", project_name);
    println!("  Target: {:?}", target);
    println!("  Output directory: {:?}", output_dir);

    let config = mobench_sdk::InitConfig {
        target: target.into(),
        project_name: project_name.clone(),
        output_dir: output_dir.clone(),
        generate_examples,
    };

    mobench_sdk::codegen::generate_project(&config).context("Failed to generate project")?;

    println!("\n✓ Project initialized successfully!");
    println!("\nNext steps:");
    println!("  1. Add benchmark functions to your code with #[benchmark]");
    println!("  2. Run 'cargo build --target <platform>' to build");
    println!("  3. Run benchmarks with 'cargo mobench build --target <platform>'");

    Ok(())
}

/// Build mobile artifacts using mobench-sdk (Phase 1 MVP)
fn cmd_build(target: SdkTarget, release: bool) -> Result<()> {
    println!("Building mobile artifacts...");
    println!("  Target: {:?}", target);
    println!("  Profile: {}", if release { "release" } else { "debug" });

    let project_root = std::env::current_dir().context("Failed to get current directory")?;
    let crate_name = detect_bench_mobile_crate_name(&project_root)
        .unwrap_or_else(|_| "bench-mobile".to_string()); // Fallback for legacy layouts

    let build_config = mobench_sdk::BuildConfig {
        target: target.into(),
        profile: if release {
            mobench_sdk::BuildProfile::Release
        } else {
            mobench_sdk::BuildProfile::Debug
        },
        incremental: true,
    };

    match target {
        SdkTarget::Android => {
            let builder =
                mobench_sdk::builders::AndroidBuilder::new(&project_root, crate_name.clone())
                    .verbose(true);
            let result = builder.build(&build_config)?;
            println!("\n✓ Android build completed!");
            println!("  APK: {:?}", result.app_path);
        }
        SdkTarget::Ios => {
            let builder = mobench_sdk::builders::IosBuilder::new(&project_root, crate_name.clone())
                .verbose(true);
            let result = builder.build(&build_config)?;
            println!("\n✓ iOS build completed!");
            println!("  Framework: {:?}", result.app_path);
        }
        SdkTarget::Both => {
            // Build Android
            let android_builder =
                mobench_sdk::builders::AndroidBuilder::new(&project_root, crate_name.clone())
                    .verbose(true);
            let android_result = android_builder.build(&build_config)?;
            println!("\n✓ Android build completed!");
            println!("  APK: {:?}", android_result.app_path);

            // Build iOS
            let ios_builder =
                mobench_sdk::builders::IosBuilder::new(&project_root, crate_name).verbose(true);
            let ios_result = ios_builder.build(&build_config)?;
            println!("\n✓ iOS build completed!");
            println!("  Framework: {:?}", ios_result.app_path);
        }
    }

    Ok(())
}

fn detect_bench_mobile_crate_name(root: &Path) -> Result<String> {
    let path = root.join("bench-mobile").join("Cargo.toml");
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("reading bench-mobile manifest at {:?}", path))?;
    let value: toml::Value = toml::from_str(&contents)
        .with_context(|| format!("parsing bench-mobile manifest {:?}", path))?;
    let name = value
        .get("package")
        .and_then(|pkg| pkg.get("name"))
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow!("bench-mobile package.name missing in {:?}", path))?;
    Ok(name.to_string())
}

/// List all discovered benchmark functions (Phase 1 MVP)
fn cmd_list() -> Result<()> {
    println!("Discovering benchmark functions...\n");

    let benchmarks = mobench_sdk::discover_benchmarks();

    if benchmarks.is_empty() {
        println!("No benchmarks found.");
        println!("\nTo add benchmarks:");
        println!("  1. Add #[benchmark] attribute to functions");
        println!("  2. Make sure mobench-sdk is in your dependencies");
        println!("  3. Rebuild your project");
    } else {
        println!("Found {} benchmark(s):", benchmarks.len());
        for bench in benchmarks {
            println!("  - {}", bench.name);
        }
    }

    Ok(())
}

/// Package iOS app as IPA for distribution or testing
fn cmd_package_ipa(scheme: &str, method: IosSigningMethodArg) -> Result<()> {
    println!("Packaging iOS app as IPA...");
    println!("  Scheme: {}", scheme);
    println!("  Method: {:?}", method);

    let project_root = std::env::current_dir().context("Failed to get current directory")?;
    let crate_name = detect_bench_mobile_crate_name(&project_root)
        .unwrap_or_else(|_| "bench-mobile".to_string());

    let builder = mobench_sdk::builders::IosBuilder::new(&project_root, crate_name).verbose(true);

    let signing_method: mobench_sdk::builders::SigningMethod = method.into();
    let ipa_path = builder
        .package_ipa(scheme, signing_method)
        .context("Failed to package IPA")?;

    println!("\n✓ IPA packaged successfully!");
    println!("  Path: {:?}", ipa_path);
    println!("\nYou can now:");
    println!("  - Install on device: Use Xcode or ios-deploy");
    println!("  - Test on BrowserStack: cargo mobench run --target ios --ios-app {:?}", ipa_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Register a lightweight benchmark for tests so the inventory contains at least one entry.
    #[mobench_sdk::benchmark]
    fn noop_benchmark() {
        std::hint::black_box(1u8);
    }

    #[test]
    fn resolves_cli_spec() {
        let spec = resolve_run_spec(
            MobileTarget::Android,
            "sample_fns::fibonacci".into(),
            5,
            1,
            vec!["pixel".into()],
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(spec.function, "sample_fns::fibonacci");
        assert_eq!(spec.iterations, 5);
        assert_eq!(spec.warmup, 1);
        assert_eq!(spec.devices, vec!["pixel".to_string()]);
        assert!(spec.browserstack.is_none());
        assert!(spec.ios_xcuitest.is_none());
    }

    #[test]
    fn local_smoke_produces_samples() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "noop_benchmark".into(),
            iterations: 3,
            warmup: 1,
            devices: vec![],
            browserstack: None,
            ios_xcuitest: None,
        };
        let report = run_local_smoke(&spec).expect("local harness");
        assert!(report["samples"].is_array());
        assert_eq!(report["spec"]["name"], "noop_benchmark");
    }

    #[test]
    fn ios_requires_artifacts_for_browserstack() {
        let err = resolve_run_spec(
            MobileTarget::Ios,
            "sample_fns::fibonacci".into(),
            1,
            0,
            vec!["iphone".into()],
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("iOS BrowserStack runs require --ios-app and --ios-test-suite")
        );
    }
}
