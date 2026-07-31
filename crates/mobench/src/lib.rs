//! # mobench
//!
//! [![Crates.io](https://img.shields.io/crates/v/mobench.svg)](https://crates.io/crates/mobench)
//! [![Documentation](https://docs.rs/mobench/badge.svg)](https://docs.rs/mobench)
//! [![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/worldcoin/mobile-bench-rs/blob/main/LICENSE)
//!
//! Command-line tool for building, running, reporting, and profiling Rust
//! benchmarks on mobile platforms.
//!
//! ## Overview
//!
//! `mobench` is the CLI orchestrator for the mobench ecosystem. It handles:
//!
//! - **Building** - Compiles Rust code for Android/iOS/WebAssembly
//! - **Running** - Executes benchmarks locally or on BrowserStack devices
//! - **Profiling** - Plans and executes supported local native captures
//! - **Reporting** - Collects and formats benchmark and profiling results
//!
//! ## Installation
//!
//! ```bash
//! cargo install mobench
//! ```
//!
//! ## Quick Start
//!
//! ```bash
//! # Initialize a benchmark project
//! cargo mobench init --target android --output bench-config.toml
//!
//! # Build for Android
//! cargo mobench build --target android
//!
//! # Build for iOS
//! cargo mobench build --target ios
//!
//! # Run locally (no device required)
//! cargo mobench run --target android --function my_benchmark --local-only
//!
//! # Run on BrowserStack (use --release for smaller APK uploads)
//! cargo mobench run --target android --function my_benchmark \
//!     --iterations 100 --warmup 10 --devices "Google Pixel 7-13.0" --release
//! ```
//!
//! ## Commands
//!
//! | Command | Description |
//! |---------|-------------|
//! | `init` | Initialize a new benchmark project |
//! | `build` | Build mobile artifacts (APK/xcframework) |
//! | `run-web` | Execute a hosted WASM bundle through BrowserStack Automate |
//! | `run` | Execute benchmarks locally or on devices |
//! | `ci prepare` | Build a hashed prebuilt bundle without credentials |
//! | `ci run-prebuilt` | Verify and run a closed bundle with BrowserStack credentials |
//! | `ci run` | Run standardized CI orchestration (`summary.json`, `summary.md`, `results.csv`) |
//! | `doctor` | Validate local/CI prerequisites and configuration |
//! | `config validate` | Validate run config + matrix contract |
//! | `devices resolve` | Resolve deterministic device sets from matrix/profile |
//! | `fixture ...` | Fixture lifecycle helpers (`init`, `build`, `verify`, `verify-plots`, `cache-key`) |
//! | `report ...` | Render markdown and publish sticky PR comments |
//! | `list` | List discovered benchmark functions |
//! | `fetch` | Retrieve results from BrowserStack |
//! | `package-ipa` | Package iOS app as IPA |
//! | `package-xcuitest` | Package XCUITest runner |
//!
//! ## Output Directory
//!
//! All build artifacts are written to `target/mobench/` by default:
//!
//! ```text
//! target/mobench/
//! ├── android/           # Android project and APK
//! └── ios/               # iOS project, xcframework, and IPA
//! ```
//!
//! Use `--output-dir` to customize the output location.
//!
//! ## Configuration
//!
//! Benchmarks can be configured via command-line arguments or a TOML config file:
//!
//! ```toml
//! target = "android"
//! function = "my_crate::my_benchmark"
//! iterations = 100
//! warmup = 10
//!
//! [browserstack]
//! app_automate_username = "${BROWSERSTACK_USERNAME}"
//! app_automate_access_key = "${BROWSERSTACK_ACCESS_KEY}"
//! project = "my-project"
//! ```
//!
//! ## BrowserStack Integration
//!
//! The CLI integrates with BrowserStack App Automate for running benchmarks
//! on real devices. Set credentials via environment variables:
//!
//! ```bash
//! export BROWSERSTACK_USERNAME="your_username"
//! export BROWSERSTACK_ACCESS_KEY="your_access_key"
//! ```
//!
//! ## Crate Ecosystem
//!
//! This crate is part of the mobench ecosystem:
//!
//! - **`mobench`** (this crate) - CLI tool
//! - **[`mobench-sdk`](https://crates.io/crates/mobench-sdk)** - Core SDK with timing harness and build automation
//! - **[`mobench-macros`](https://crates.io/crates/mobench-macros)** - `#[benchmark]` proc macro
//!
//! Note: The `mobench-runner` crate has been consolidated into `mobench-sdk` as its `timing` module.
//!
//! ## CLI Flags
//!
//! Global flags available on all commands:
//!
//! - **`--dry-run`** - Preview what would be done without making changes
//! - **`--verbose` / `-v`** - Enable detailed output showing all commands
//!
//! ## Modules
//!
//! - [`config`] - Configuration file support for `mobench.toml`

#![cfg_attr(docsrs, feature(doc_cfg))]

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
#[cfg(test)]
use mobench_report::{
    BenchmarkFailureStats, BenchmarkResourceUsage, BenchmarkStats, CompareReport, CompareRow,
    DeviceSummary, MEMORY_BASELINE_GAP_NOTE, format_cpu_total_duration_ms, format_duration_smart,
    format_ms, render_csv_summary, render_markdown_summary,
};
use mobench_report::{
    RegressionFinding, SummaryReport as CanonicalSummaryReport, detect_regressions,
    render_compare_markdown,
};
#[cfg(test)]
use mobench_runtime::MAX_BENCHMARK_COUNT;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

use browserstack::{
    BrowserStackAuth, BrowserStackClient, BrowserStackPlatform, BrowserStackProviderAdapter,
    BrowserStackRunHandle, completed_browserstack_collection,
};
use build_commands::*;
use ci_prebuilt::{cmd_ci_prepare, cmd_ci_run_prebuilt};
#[cfg(test)]
pub(crate) use cli::CiTarget;
pub use cli::MobileTarget;
pub(crate) use cli::{
    BuildTarget, CheckOutputFormat, CiCommand, CiMergeSplitRunsArgs, Cli, Command, ConfigCommand,
    ContractErrorCategory, DevicePlatform, DevicesCommand, FixtureCommand, IosRunnerArg,
    ProfileCommand, ReportCommand, SdkTarget,
};
use devices::*;
#[cfg(test)]
pub(crate) use doctor::{
    DEFAULT_ANDROID_DOCTOR_RUST_TARGETS, WORKSPACE_MSRV, category_slug, parse_rust_version,
    render_check_results_json, rustc_version_meets_msrv,
};
#[cfg(test)]
use doctor::{PrereqCheck, collect_issues};
pub(crate) use doctor::{cmd_check, cmd_config_validate, cmd_doctor};
use fixtures::*;
use local_provider::{LocalProviderAdapter, LocalRunRequest};
use project_layout::*;
pub(crate) use report_binding::RunEnvelopeIdentity;
use report_binding::bind_report_value;
use reporting::*;
use run_spec::*;
use summary_command::*;
use web_commands::cmd_run_web;
use workspace_fs::*;

mod browserstack;
pub(crate) mod browserstack_automate;
mod build_commands;
mod ci;
mod ci_prebuilt;
mod cli;
pub mod config;
mod devices;
mod doctor;
mod execution;
mod fixtures;
mod flamegraph_viewer;
mod github;
mod local_provider;
mod plots;
mod process_adapter;
mod profile;
mod project_layout;
mod report_binding;
mod reporting;
mod run_lifecycle;
mod run_spec;
mod split_runs;
pub(crate) mod summarize;
mod summary_command;
mod web_commands;
mod workspace_fs;

pub use ci::*;
pub use execution::*;

/// Install the CLI's cooperative interrupt handler once.
///
/// The first Ctrl-C cancels bounded child-process scopes so they can be killed
/// and reaped. A second Ctrl-C exits immediately with the conventional code.
pub fn install_interrupt_handler() -> Result<()> {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static INSTALLED: OnceLock<()> = OnceLock::new();
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    let cancellation = mobench_process::global_cancellation_token();
    let signals = std::sync::Arc::new(AtomicUsize::new(0));
    ctrlc::set_handler(move || {
        if signals.fetch_add(1, Ordering::SeqCst) == 0 {
            cancellation.cancel();
        } else {
            std::process::exit(130);
        }
    })
    .context("installing Ctrl-C handler")?;
    let _ = INSTALLED.set(());
    Ok(())
}

/// Whether the CLI received a cooperative interruption request.
pub fn interruption_requested() -> bool {
    mobench_process::global_cancellation_token().is_cancelled()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BrowserStackConfig {
    app_automate_username: String,
    app_automate_access_key: String,
    project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    ios_completion_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    android_benchmark_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    android_heartbeat_interval_secs: Option<u64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_tags: Option<Vec<String>>,
    browserstack: BrowserStackConfig,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    ios_xcuitest: Option<IosXcuitestArtifacts>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct DeviceEntry {
    name: String,
    os: String,
    #[serde(default)]
    os_version: String,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeviceMatrix {
    devices: Vec<DeviceEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct RunSpec {
    pub(crate) target: MobileTarget,
    pub(crate) function: String,
    pub(crate) iterations: u32,
    pub(crate) warmup: u32,
    pub(crate) devices: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) ios_completion_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) ios_deployment_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) ios_runner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) android_benchmark_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) android_heartbeat_interval_secs: Option<u64>,
    #[serde(skip_serializing, skip_deserializing, default)]
    pub(crate) browserstack: Option<BrowserStackConfig>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub(crate) ios_xcuitest: Option<IosXcuitestArtifacts>,
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
    summary: SummaryReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    benchmark_results: Option<BTreeMap<String, Vec<Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    benchmark_failures: Option<BTreeMap<String, Vec<Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    performance_metrics: Option<BTreeMap<String, browserstack::PerformanceMetrics>>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedProjectLayout {
    pub(crate) project_root: PathBuf,
    pub(crate) crate_dir: PathBuf,
    pub(crate) crate_name: String,
    pub(crate) library_name: String,
    pub(crate) ffi_backend: mobench_sdk::FfiBackend,
    pub(crate) android_abis: Option<Vec<String>>,
    pub(crate) ios_completion_timeout_secs: Option<u64>,
    pub(crate) ios_deployment_target: String,
    pub(crate) ios_runner: Option<String>,
    pub(crate) android_benchmark_timeout_secs: Option<u64>,
    pub(crate) android_heartbeat_interval_secs: Option<u64>,
    pub(crate) web_wasm_bindgen: Option<PathBuf>,
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) output_dir: PathBuf,
    pub(crate) default_function: Option<String>,
}

type SummaryReport = CanonicalSummaryReport<MobileTarget>;

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

fn init_tracing(verbose: bool) {
    let filter = env::var("MOBENCH_LOG").unwrap_or_else(|_| {
        if verbose {
            "mobench=debug".to_string()
        } else {
            "warn".to_string()
        }
    });
    let filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("warn"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .without_time()
        .try_init();
}

pub fn run() -> Result<()> {
    // Load dotenv globally as a baseline for commands that don't resolve a layout
    // (e.g. fetch, doctor, ci run). Layout-aware commands reload from the resolved root.
    load_dotenv_global();
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    debug!(
        dry_run = cli.dry_run,
        non_interactive = cli.non_interactive,
        "parsed CLI arguments"
    );
    match cli.command {
        Command::Run {
            target,
            function,
            project_root,
            crate_path,
            iterations,
            warmup,
            devices,
            device_matrix,
            device_tags,
            config,
            output,
            summary_csv,
            ci,
            baseline,
            regression_threshold_pct,
            junit,
            local_only,
            release,
            ios_app,
            ios_test_suite,
            ios_completion_timeout_secs,
            ios_deployment_target,
            ios_runner,
            android_benchmark_timeout_secs,
            android_heartbeat_interval_secs,
            fetch,
            fetch_output_dir,
            fetch_poll_interval_secs,
            fetch_timeout_secs,
            progress,
        } => {
            let layout = resolve_project_layout(ProjectLayoutOptions {
                start_dir: None,
                project_root: project_root.as_deref(),
                crate_path: crate_path.as_deref(),
                config_path: config.as_deref(),
            })?;
            load_dotenv_for_layout(&layout);
            let spec = resolve_run_spec(
                target,
                function,
                iterations,
                warmup,
                devices,
                &layout,
                config.as_deref(),
                device_matrix.as_deref(),
                device_tags,
                ios_app,
                ios_test_suite,
                ios_completion_timeout_secs,
                ios_deployment_target,
                ios_runner,
                android_benchmark_timeout_secs,
                android_heartbeat_interval_secs,
                local_only,
                release,
                cli.dry_run,
            )?;
            let run_identity = RunEnvelopeIdentity::generate(spec.target)?;
            let summary_paths = resolve_summary_paths(output.as_deref())?;
            let lifecycle_expected_sessions = if local_only {
                vec!["host".to_owned()]
            } else if fetch {
                spec.devices.clone()
            } else {
                Vec::new()
            };
            let run_lifecycle = run_lifecycle::ResolvedRunPlan::new(
                run_identity.run_id.clone(),
                spec.target,
                spec.function.clone(),
                mobench_domain::ReportCounts::new(spec.iterations, spec.warmup)
                    .context("invalid resolved run counts")?,
                lifecycle_expected_sessions,
            )?
            .begin();
            let output_dir = layout.output_dir.clone();
            let run_span = tracing::info_span!(
                "benchmark_run",
                target = ?spec.target,
                function = %spec.function,
                iterations = spec.iterations,
                warmup = spec.warmup,
                devices = spec.devices.len(),
                local_only,
                release
            );
            let _run_span = run_span.enter();
            info!(
                output_dir = %output_dir.display(),
                summary_json = %summary_paths.json.display(),
                "resolved benchmark run"
            );

            // Validate device specs early to catch errors before building (C2: Device validation)
            if !spec.devices.is_empty() && !local_only {
                info!(device_count = spec.devices.len(), "validating device specs");
                if cli.dry_run {
                    println!("[dry-run] Skipping BrowserStack device validation");
                } else if let Ok(creds) =
                    resolve_browserstack_credentials(spec.browserstack.as_ref())
                {
                    let client = BrowserStackClient::new(
                        BrowserStackAuth {
                            username: creds.username,
                            access_key: creds.access_key,
                        },
                        creds.project,
                    )?;

                    let platform_str = match spec.target {
                        MobileTarget::Android => Some("android"),
                        MobileTarget::Ios => Some("ios"),
                    };

                    println!("Validating device specifications...");
                    let validation = client.validate_devices(&spec.devices, platform_str)?;

                    if !validation.invalid.is_empty() {
                        println!();
                        println!("Invalid device specifications:");
                        for error in &validation.invalid {
                            println!("  [ERROR] {}: {}", error.spec, error.reason);
                            if !error.suggestions.is_empty() {
                                println!("          Did you mean:");
                                for suggestion in &error.suggestions {
                                    println!("            - {}", suggestion);
                                }
                            }
                        }
                        println!();
                        println!("Use 'cargo mobench devices' to see available devices.");
                        bail!(
                            "{} of {} device specs are invalid. Fix them before running.",
                            validation.invalid.len(),
                            spec.devices.len()
                        );
                    }
                    if spec.target == MobileTarget::Ios
                        && let Some(deployment_target) = spec.ios_deployment_target.as_deref()
                    {
                        let parsed_deployment_target =
                            mobench_sdk::codegen::IosDeploymentTarget::parse(deployment_target)
                                .map_err(|err| anyhow!("config_error: {err}"))?;
                        validate_ios_device_specs_support_deployment_target(
                            &validation.valid,
                            &parsed_deployment_target,
                        )?;
                    }
                    println!(
                        "  All {} device(s) validated successfully.",
                        validation.valid.len()
                    );
                }
            }

            // Print resolved spec summary (A5: Better CLI output)
            if !progress {
                println!();
                println!("=== Benchmark Run Configuration ===");
                println!("  Target:      {:?}", spec.target);
                println!("  Function:    {}", spec.function);
                println!("  Iterations:  {}", spec.iterations);
                println!("  Warmup:      {}", spec.warmup);
                println!(
                    "  Profile:     {}",
                    if release { "release" } else { "debug" }
                );
                if cli.dry_run {
                    println!("  Mode:        dry-run");
                }
                if !spec.devices.is_empty() {
                    println!("  Devices:     {}", spec.devices.join(", "));
                } else {
                    println!("  Devices:     (none - local build only)");
                }
                println!();

                // Print artifact locations
                println!("=== Output Locations ===");
                println!("  Build output:    {}", output_dir.display());
                match spec.target {
                    MobileTarget::Android => {
                        println!(
                            "  Android APK:     {}/android/app/build/outputs/apk/",
                            output_dir.display()
                        );
                        println!(
                            "  bench_spec.json: {}/android/app/src/main/assets/",
                            output_dir.display()
                        );
                    }
                    MobileTarget::Ios => {
                        println!("  iOS xcframework: {}/ios/", output_dir.display());
                        println!(
                            "  bench_spec.json: {}/ios/BenchRunner/BenchRunner/Resources/",
                            output_dir.display()
                        );
                        if let Some(ref xcui) = spec.ios_xcuitest {
                            println!("  iOS App IPA:     {}", xcui.app.display());
                            println!("  XCUITest Runner: {}", xcui.test_suite.display());
                        }
                    }
                }
                println!("  JSON summary:    {}", summary_paths.json.display());
                println!("  Markdown:        {}", summary_paths.markdown.display());
                if summary_csv {
                    println!("  CSV:             {}", summary_paths.csv.display());
                }
                println!();
            }

            // A2: Validate that the requested benchmark function exists (if we can detect it)
            if !progress {
                validate_benchmark_function(&layout, &spec.function)?;
            }

            // Persist the spec and metadata to mobile app bundles
            if progress {
                println!("[1/4] Preparing benchmark spec...");
            }
            if cli.dry_run {
                println!(
                    "[dry-run] Would write bench_spec.json and bench_meta.json under {}",
                    output_dir.display()
                );
            } else {
                persist_mobile_spec(&layout, &spec, &run_identity, release)?;
            }

            let mut bound_reports = Vec::new();
            let local_report = if local_only && !cli.dry_run {
                if progress {
                    println!("[2/4] Running benchmark through local provider...");
                } else {
                    println!("Running host benchmark through local provider...");
                }
                let engine =
                    mobench_provider::ProviderEngine::new(LocalProviderAdapter::new(&layout));
                let request = LocalRunRequest {
                    function: spec.function.clone(),
                    iterations: spec.iterations,
                    warmup: spec.warmup,
                    release,
                    run_id: run_identity.run_id.clone(),
                    nonce: run_identity.nonce.clone(),
                    logical_session_id: run_identity.logical_session_id.clone(),
                    producer: run_identity.producer.clone(),
                };
                let run = engine
                    .execute(&request, &mobench_process::global_cancellation_token())
                    .map_err(|error| anyhow!("local provider failed: {error}"))?;
                if !run.assessment().is_complete() {
                    bail!(
                        "local provider returned an incomplete run: {}",
                        run.assessment()
                    );
                }
                let session = run
                    .sessions()
                    .first()
                    .context("local provider completed without one session")?;
                let report = session
                    .reports
                    .first()
                    .cloned()
                    .context("local provider completed without one benchmark report")?;
                bound_reports.push(bind_report_value(
                    &report,
                    &run_identity,
                    &spec,
                    "local",
                    &run_identity.run_id,
                    &session.session_id,
                    "host",
                    "host",
                )?);
                report
            } else {
                if !progress && !local_only {
                    println!("Skipping host benchmark - benchmarks will run on mobile devices");
                }
                json!({
                    "skipped": true,
                    "reason": if cli.dry_run {
                        "Local provider skipped during dry-run"
                    } else {
                        "Host benchmark disabled - benchmarks run on mobile devices"
                    }
                })
            };
            let mut remote_run = None;
            let artifacts = if local_only {
                if !progress {
                    println!("Skipping mobile build: --local-only set");
                }
                None
            } else {
                match spec.target {
                    MobileTarget::Android => {
                        info!("building Android artifacts");
                        if progress {
                            println!("[2/4] Building Android APK...");
                        } else {
                            println!("Building for Android...");
                            println!("  Building Rust library for Android targets...");
                        }
                        let ndk = std::env::var("ANDROID_NDK_HOME").context(
                            "ANDROID_NDK_HOME must be set for Android builds. Example: export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/<version>",
                        )?;
                        let build = run_android_build(&layout, &ndk, release, cli.dry_run)?;
                        let apk = build.app_path;
                        if !progress {
                            println!("\u{2713} Built Android APK at {:?}", apk);
                        }
                        if spec.devices.is_empty() {
                            if !progress {
                                println!("Skipping BrowserStack upload/run: no devices provided");
                            }
                            Some(MobileArtifacts::Android { apk })
                        } else if cli.dry_run {
                            if !progress {
                                println!("[dry-run] Skipping BrowserStack upload/run for Android");
                            }
                            Some(MobileArtifacts::Android { apk })
                        } else {
                            info!("uploading Android artifacts to BrowserStack");
                            if progress {
                                println!("[3/4] Uploading to BrowserStack...");
                            }
                            let test_apk = build.test_suite_path.as_ref().context(
                                "Android test suite APK missing. Run `cargo mobench build --target android` or `./gradlew app:assembleReleaseAndroidTest` in target/mobench/android",
                            )?;
                            let run = trigger_browserstack_espresso(&spec, &apk, test_apk)?;
                            remote_run = Some(run);
                            Some(MobileArtifacts::Android { apk })
                        }
                    }
                    MobileTarget::Ios => {
                        info!("building iOS artifacts");
                        if progress {
                            println!("[2/4] Building iOS xcframework...");
                        } else {
                            println!("Building for iOS...");
                            println!("  Building Rust library for iOS targets...");
                        }
                        let (xcframework, header) = run_ios_build(
                            &layout,
                            release,
                            cli.dry_run,
                            spec.ios_completion_timeout_secs,
                            spec.ios_deployment_target.as_deref(),
                            spec.ios_runner.as_deref(),
                        )?;
                        if !progress {
                            println!("\u{2713} Built iOS xcframework at {:?}", xcframework);
                        }
                        let mut ios_xcuitest = spec.ios_xcuitest.clone();

                        if spec.devices.is_empty() {
                            if !progress {
                                println!("Skipping BrowserStack upload/run: no devices provided");
                            }
                        } else if cli.dry_run {
                            if !progress {
                                println!("[dry-run] Skipping BrowserStack upload/run for iOS");
                            }
                        } else {
                            info!("uploading iOS artifacts to BrowserStack");
                            if ios_xcuitest.as_ref().is_some_and(|artifacts| {
                                uses_managed_ios_xcuitest_artifacts(&layout, artifacts)
                            }) {
                                println!(
                                    "📦 Packaging iOS BrowserStack artifacts with current bench_spec..."
                                );
                                let packaged = package_ios_xcuitest_artifacts(
                                    &layout,
                                    &spec,
                                    &run_identity,
                                    release,
                                    spec.ios_completion_timeout_secs,
                                    spec.ios_deployment_target.as_deref(),
                                    spec.ios_runner.as_deref(),
                                )?;
                                println!("  ✓ IPA: {}", packaged.app.display());
                                println!("  ✓ XCUITest: {}", packaged.test_suite.display());
                                ios_xcuitest = Some(packaged);
                            }
                            if progress {
                                println!("[3/4] Uploading to BrowserStack...");
                            }
                            let xcui = ios_xcuitest.as_ref().context(
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

            let summary_placeholder = empty_summary(&spec);
            let mut run_summary = RunSummary {
                spec,
                artifacts,
                local_report,
                remote_run,
                summary: summary_placeholder,
                benchmark_results: None,
                benchmark_failures: None,
                performance_metrics: None,
            };

            if cli.dry_run {
                println!();
                println!("[dry-run] Run simulation completed. No changes were made.");
                return Ok(());
            }

            let mut pending_browserstack_error: Option<String> = None;
            if fetch && let Some(remote) = &run_summary.remote_run {
                let build_id = match remote {
                    RemoteRun::Android { build_id, .. } => build_id,
                    RemoteRun::Ios { build_id, .. } => build_id,
                };
                let creds =
                    resolve_browserstack_credentials(run_summary.spec.browserstack.as_ref())?;
                let client = BrowserStackClient::new(
                    BrowserStackAuth {
                        username: creds.username,
                        access_key: creds.access_key,
                    },
                    creds.project,
                )?;

                let provider_handle = match remote {
                    RemoteRun::Android { app_url, build_id } => BrowserStackRunHandle {
                        platform: BrowserStackPlatform::Espresso,
                        requested_devices: run_summary.spec.devices.clone(),
                        app_url: app_url.clone(),
                        test_suite_url: None,
                        build_id: build_id.clone(),
                    },
                    RemoteRun::Ios {
                        app_url,
                        test_suite_url,
                        build_id,
                    } => BrowserStackRunHandle {
                        platform: BrowserStackPlatform::XcuiTest,
                        requested_devices: run_summary.spec.devices.clone(),
                        app_url: app_url.clone(),
                        test_suite_url: Some(test_suite_url.clone()),
                        build_id: build_id.clone(),
                    },
                };

                let dashboard_url = format!(
                    "https://app-automate.browserstack.com/dashboard/v2/builds/{}",
                    build_id
                );

                println!("Waiting for build {} to complete...", build_id);
                println!("Dashboard: {}", dashboard_url);

                let mut browserstack_artifacts_fetched = false;
                let provider =
                    mobench_provider::ProviderEngine::new(BrowserStackProviderAdapter::new(
                        client.clone(),
                        fetch_timeout_secs,
                        fetch_poll_interval_secs,
                    ));
                let provider_result = provider
                    .collect(
                        mobench_provider::StartedRun::from_handle(provider_handle),
                        &mobench_process::global_cancellation_token(),
                    )
                    .map_err(anyhow::Error::new)
                    .context("BrowserStack provider failed to collect")
                    .and_then(completed_browserstack_collection);
                match provider_result {
                    Ok(collection) => {
                        for collected in &collection.reports {
                            bound_reports.push(bind_report_value(
                                &collected.benchmark,
                                &run_identity,
                                &run_summary.spec,
                                "browserstack",
                                build_id,
                                &collected.transport_session_id,
                                &collected.requested_device_id,
                                &collected.observed_device_id,
                            )?);
                        }
                        let bench_results = collection.benchmark_results;
                        let perf_metrics = collection.performance_metrics;
                        println!(
                            "\n✓ Successfully fetched results from {} device(s)",
                            bench_results.len()
                        );

                        // Print summary of benchmark results
                        for (device, results) in &bench_results {
                            println!("\n  Device: {}", device);
                            for (idx, result) in results.iter().enumerate() {
                                if let Some(function) =
                                    result.get("function").and_then(|f| f.as_str())
                                {
                                    println!("    Benchmark {}: {}", idx + 1, function);
                                }
                                if let Some(mean) = result.get("mean_ns").and_then(|m| m.as_u64()) {
                                    println!(
                                        "      Mean: {} ns ({:.2} ms)",
                                        mean,
                                        mean as f64 / 1_000_000.0
                                    );
                                }
                                if let Some(samples) =
                                    result.get("samples").and_then(|s| s.as_array())
                                {
                                    println!("      Samples: {}", samples.len());
                                }
                            }

                            // Print performance metrics if available
                            if let Some(metrics) =
                                perf_metrics.get(device).filter(|m| m.sample_count > 0)
                            {
                                println!("\n    Performance Metrics:");
                                if let Some(mem) = &metrics.memory {
                                    println!("      Memory:");
                                    println!("        Peak: {:.2} MB", mem.peak_mb);
                                    println!("        Average: {:.2} MB", mem.average_mb);
                                }
                                if let Some(cpu) = &metrics.cpu {
                                    println!("      CPU:");
                                    println!("        Peak: {:.1}%", cpu.peak_percent);
                                    println!("        Average: {:.1}%", cpu.average_percent);
                                }
                            }
                        }

                        println!("\n  View full results: {}", dashboard_url);
                        run_summary.benchmark_results = Some(bench_results.into_iter().collect());
                        run_summary.performance_metrics = Some(perf_metrics.into_iter().collect());
                    }
                    Err(e) => {
                        let output_root = fetch_output_dir.join(build_id);
                        if let Err(fetch_err) = fetch_browserstack_artifacts(
                            &client,
                            run_summary.spec.target,
                            build_id,
                            &output_root,
                            false,
                            fetch_poll_interval_secs,
                            fetch_timeout_secs,
                        ) {
                            eprintln!(
                                "Warning: failed to fetch detailed BrowserStack artifacts after benchmark failure: {fetch_err}"
                            );
                        } else if let Ok(failures) = load_browserstack_failure_reports(&output_root)
                            && !failures.is_empty()
                        {
                            run_summary.benchmark_failures = Some(failures);
                        }
                        browserstack_artifacts_fetched = true;
                        pending_browserstack_error = Some(format!(
                            "failed to fetch BrowserStack benchmark results: {}. Build may still be accessible at: {}",
                            e, dashboard_url
                        ));
                    }
                }

                // Also save detailed artifacts to separate directory
                let output_root = fetch_output_dir.join(build_id);
                if !browserstack_artifacts_fetched {
                    fetch_browserstack_artifacts(
                        &client,
                        run_summary.spec.target,
                        build_id,
                        &output_root,
                        false, // Don't wait again, we already did
                        fetch_poll_interval_secs,
                        fetch_timeout_secs,
                    )
                    .with_context(|| {
                        format!(
                            "failed to fetch detailed BrowserStack artifacts for build {}",
                            build_id
                        )
                    })?;
                }
            } else if fetch {
                println!("No BrowserStack run to fetch (devices not provided?)");
            }

            let collected_run = run_lifecycle.collect(bound_reports)?;

            let mut baseline_compare_path = None;
            let mut baseline_snapshot_path = None;
            if let Some(baseline_source) = baseline.as_deref() {
                let resolved_baseline = resolve_baseline_source(baseline_source)?;
                if paths_point_to_same_file(&resolved_baseline, &summary_paths.json)? {
                    if !resolved_baseline.exists() {
                        bail!(
                            "config_error: baseline source `{}` resolves to output path {}; provide an existing baseline file or a different path",
                            baseline_source,
                            summary_paths.json.display()
                        );
                    }
                    let snapshot = snapshot_baseline_for_compare(&resolved_baseline)?;
                    baseline_snapshot_path = Some(snapshot.clone());
                    baseline_compare_path = Some(snapshot);
                } else {
                    baseline_compare_path = Some(resolved_baseline);
                }
            }

            run_summary.summary = build_summary(&run_summary)?;

            let mut compare_report = None;
            let mut regression_findings: Vec<RegressionFinding> = Vec::new();
            if let Some(baseline_path) = baseline_compare_path.as_deref() {
                let baseline_summary = load_run_summary(baseline_path)?;
                let report = compare_run_summaries(
                    baseline_path,
                    &summary_paths.json,
                    &baseline_summary,
                    &run_summary,
                );
                regression_findings = detect_regressions(&report, regression_threshold_pct);
                compare_report = Some(report);
            }
            if let Some(snapshot_path) = baseline_snapshot_path
                && let Err(err) = fs::remove_file(&snapshot_path)
            {
                eprintln!(
                    "Warning: failed to remove baseline snapshot {}: {err}",
                    snapshot_path.display()
                );
            }

            info!("preparing benchmark summaries");
            let (mut summary_value, markdown, csv) = prepare_summary_artifacts(
                &run_summary,
                &summary_paths,
                summary_csv,
                plots::PlotMode::Off,
            )?;
            if let Some(report) = &compare_report {
                inject_compare_into_summary_value(
                    &mut summary_value,
                    report,
                    regression_threshold_pct,
                    baseline.as_deref(),
                );
            }
            let canonical_path = summary_paths.json.with_file_name("summary.v2.json");
            let report_artifacts = run_lifecycle::ReportArtifacts::new(
                &summary_paths.json,
                summary_value,
                &summary_paths.markdown,
                markdown,
                csv.map(|contents| (summary_paths.csv.as_path(), contents)),
                &canonical_path,
            )?;
            let committed_run = collected_run.prepare(report_artifacts)?.commit()?;
            info!(
                publication = %committed_run.published_path().display(),
                outcome = ?committed_run.outcome(),
                stable_artifacts = committed_run.stable_paths().len(),
                "committed benchmark report publication"
            );
            println!("Wrote run summary to {:?}", summary_paths.json);
            println!("Wrote markdown summary to {:?}", summary_paths.markdown);
            if summary_csv {
                println!("Wrote CSV summary to {:?}", summary_paths.csv);
            }
            println!("Wrote canonical v2 summary to {:?}", canonical_path);

            if !committed_run.outcome().is_complete() && pending_browserstack_error.is_none() {
                pending_browserstack_error = Some(format!(
                    "benchmark run did not complete every expected session: {:?}",
                    committed_run.outcome()
                ));
            }

            if ci {
                if let Err(err) = append_github_step_summary_from_path(&summary_paths.markdown) {
                    eprintln!("Warning: failed to publish job summary: {err}");
                }
                if let Some(report) = &compare_report {
                    let compare_markdown = render_compare_markdown(report);
                    if let Ok(summary_path) = env::var("GITHUB_STEP_SUMMARY")
                        && let Err(err) =
                            append_github_step_summary(&compare_markdown, &summary_path)
                    {
                        eprintln!("Warning: failed to append comparison report: {err}");
                    }
                }
            } else if let Some(report) = &compare_report {
                println!(
                    "{compare_markdown}",
                    compare_markdown = render_compare_markdown(report)
                );
            }

            if let Some(junit_path) = junit.as_deref() {
                write_junit_report(junit_path, &run_summary.summary, &regression_findings)?;
            }

            if let Some(error) = pending_browserstack_error {
                println!();
                println!("Results saved to:");
                println!("  * {} (machine-readable)", summary_paths.json.display());
                println!("  * {} (human-readable)", summary_paths.markdown.display());
                if summary_csv {
                    println!("  * {} (spreadsheet)", summary_paths.csv.display());
                }
                bail!("{error}");
            }

            // Print clear completion summary
            println!();
            println!("\u{2713} Benchmark complete!");
            println!();
            println!("Results saved to:");
            println!("  * {} (machine-readable)", summary_paths.json.display());
            println!("  * {} (human-readable)", summary_paths.markdown.display());
            if summary_csv {
                println!("  * {} (spreadsheet)", summary_paths.csv.display());
            }
            println!();
            println!(
                "View results: cat {} | jq '.summary'",
                summary_paths.json.display()
            );

            if !regression_findings.is_empty() {
                eprintln!();
                eprintln!(
                    "Detected {} performance regression(s) above {:.2}% threshold.",
                    regression_findings.len(),
                    regression_threshold_pct
                );
                for finding in &regression_findings {
                    eprintln!(
                        "  - {} :: {} ({}) {:+.2}%",
                        finding.device, finding.function, finding.metric, finding.delta_pct
                    );
                }
                std::process::exit(EXIT_REGRESSION);
            }
        }
        Command::Init { output, target } => {
            write_config_template(&output, target, cli.yes)?;
            println!("Wrote starter config to {:?}", output);
        }
        Command::Plan { output } => {
            write_device_matrix_template(&output, cli.yes)?;
            println!("Wrote sample device matrix to {:?}", output);
        }
        Command::Config { command } => match command {
            ConfigCommand::Validate { config, format } => {
                cmd_config_validate(&config, format)?;
            }
        },
        Command::Doctor {
            target,
            config,
            device_matrix,
            device_tags,
            browserstack,
            format,
        } => {
            cmd_doctor(
                target,
                config.as_deref(),
                device_matrix.as_deref(),
                device_tags,
                browserstack,
                format,
            )?;
        }
        Command::Ci { command } => match command {
            CiCommand::Init {
                workflow,
                action_dir,
            } => {
                cmd_ci_init(&workflow, &action_dir, cli.yes)?;
            }
            CiCommand::Run(args) => {
                cmd_ci_run(args)?;
            }
            CiCommand::Prepare(args) => {
                cmd_ci_prepare(args, cli.dry_run)?;
            }
            CiCommand::RunPrebuilt(args) => {
                cmd_ci_run_prebuilt(args, cli.dry_run)?;
            }
            CiCommand::MergeSplitRuns(args) => {
                split_runs::cmd_ci_merge_split_runs(args, cli.dry_run)?;
            }
            CiCommand::Summarize(args) => {
                cmd_ci_summarize(args)?;
            }
            CiCommand::CheckRun(args) => {
                cmd_ci_check_run(args)?;
            }
        },
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
        Command::Compare {
            baseline,
            candidate,
            output,
        } => {
            let report = compare_summaries(&baseline, &candidate)?;
            write_compare_report(&report, output.as_deref())?;
        }
        Command::InitSdk {
            target,
            project_name,
            output_dir,
            examples,
        } => {
            cmd_init_sdk(target, project_name, output_dir, examples)?;
        }
        Command::Build {
            target,
            release,
            ios_completion_timeout_secs,
            ios_deployment_target,
            ios_runner,
            project_root,
            output_dir,
            crate_path,
            progress,
        } => {
            if target == BuildTarget::Web {
                cmd_build_web(
                    release,
                    project_root,
                    output_dir,
                    crate_path,
                    cli.dry_run,
                    cli.verbose,
                    progress,
                )?;
            } else {
                cmd_build(
                    target
                        .mobile()
                        .expect("non-web build target must map to a mobile target"),
                    release,
                    ios_completion_timeout_secs,
                    ios_deployment_target,
                    ios_runner,
                    project_root,
                    output_dir,
                    crate_path,
                    cli.dry_run,
                    cli.verbose,
                    progress,
                )?;
            }
        }
        Command::RunWeb {
            url,
            function,
            iterations,
            warmup,
            browser,
            browser_version,
            os,
            os_version,
            device,
            build_name,
            session_name,
            local_identifier,
            script_timeout_secs,
            page_load_timeout_secs,
            output,
        } => {
            cmd_run_web(
                url,
                function,
                iterations,
                warmup,
                browser,
                browser_version,
                os,
                os_version,
                device,
                build_name,
                session_name,
                local_identifier,
                script_timeout_secs,
                page_load_timeout_secs,
                output,
                cli.dry_run,
            )?;
        }
        Command::PackageIpa {
            scheme,
            method,
            project_root,
            crate_path,
            output_dir,
        } => {
            cmd_package_ipa(&scheme, method, project_root, crate_path, output_dir)?;
        }
        Command::PackageXcuitest {
            scheme,
            project_root,
            crate_path,
            output_dir,
        } => {
            cmd_package_xcuitest(&scheme, project_root, crate_path, output_dir)?;
        }
        Command::List {
            project_root,
            crate_path,
        } => {
            cmd_list(project_root, crate_path)?;
        }
        Command::Verify {
            project_root,
            crate_path,
            target,
            spec_path,
            check_artifacts,
            smoke_test,
            function,
            output_dir,
        } => {
            cmd_verify(
                project_root,
                crate_path,
                target,
                spec_path,
                check_artifacts,
                smoke_test,
                function,
                output_dir,
            )?;
        }
        Command::Summary { report, format } => {
            cmd_summary(&report, format)?;
        }
        Command::Devices {
            command,
            platform,
            json,
            validate,
        } => match command {
            Some(DevicesCommand::Resolve {
                platform,
                profile,
                config,
                device_matrix,
                format,
            }) => {
                cmd_devices_resolve(
                    platform,
                    profile,
                    config.as_deref(),
                    device_matrix.as_deref(),
                    format,
                )?;
            }
            None => {
                cmd_devices(platform, json, validate)?;
            }
        },
        Command::Fixture { command } => match command {
            FixtureCommand::Init {
                config,
                device_matrix,
                force,
            } => {
                cmd_fixture_init(&config, &device_matrix, force)?;
            }
            FixtureCommand::Build {
                target,
                release,
                output_dir,
                crate_path,
                progress,
            } => {
                cmd_fixture_build(target, release, output_dir, crate_path, progress)?;
            }
            FixtureCommand::Verify {
                config,
                device_matrix,
                target,
                profile,
                format,
            } => {
                cmd_fixture_verify(&config, device_matrix.as_deref(), target, profile, format)?;
            }
            FixtureCommand::VerifyPlots {
                fixture,
                output_dir,
            } => {
                cmd_fixture_verify_plots(fixture, output_dir.as_deref())?;
            }
            FixtureCommand::CacheKey {
                config,
                device_matrix,
                target,
                profile,
                format,
            } => {
                cmd_fixture_cache_key(&config, device_matrix.as_deref(), target, profile, format)?;
            }
        },
        Command::Report { command } => match command {
            ReportCommand::Summarize {
                summary,
                output,
                plots,
            } => {
                cmd_report_summarize(&summary, output.as_deref(), plots)?;
            }
            ReportCommand::Github {
                pr,
                summary,
                marker,
                publish,
                output,
            } => {
                cmd_report_github(pr, &summary, &marker, publish, output.as_deref())?;
            }
        },
        Command::Profile { command } => match command {
            ProfileCommand::Run(args) => {
                profile::cmd_profile_run(&args, cli.dry_run)?;
            }
            ProfileCommand::Diff(args) => {
                profile::cmd_profile_diff(&args)?;
            }
            ProfileCommand::Summarize(args) => {
                profile::cmd_profile_summarize(&args)?;
            }
        },
        Command::Check { target, format } => {
            cmd_check(target, format)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use jsonschema::JSONSchema;
    use proptest::prelude::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn render_profile_run_help() -> String {
        let mut root = Cli::command();
        let profile = root
            .find_subcommand_mut("profile")
            .expect("profile subcommand");
        let run = profile
            .find_subcommand_mut("run")
            .expect("profile run subcommand");
        let mut buffer = Vec::new();
        run.write_long_help(&mut buffer)
            .expect("render profile run help");
        String::from_utf8(buffer).expect("help is utf-8")
    }

    #[cfg(unix)]
    pub(crate) fn write_fake_plot_python(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join("fake-python");
        std::fs::write(
            &path,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  exit 0
fi

output=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then
    shift
    output="$1"
  fi
  shift
done

mkdir -p "$(dirname "$output")"
printf '<svg>ok</svg>' > "$output"
"#,
        )
        .expect("write fake python");

        let mut permissions = std::fs::metadata(&path)
            .expect("fake python metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("set fake python perms");
        path
    }

    fn write_custom_layout_project(temp_dir: &TempDir) -> (PathBuf, PathBuf) {
        let project_root = temp_dir.path().to_path_buf();
        let crate_dir = project_root.join("crates/zk-mobile-bench");

        fs::create_dir_all(crate_dir.join("src")).expect("create custom crate dir");
        write_file(
            &project_root.join("Cargo.toml"),
            br#"[workspace]
members = ["crates/zk-mobile-bench"]
resolver = "2"
"#,
        )
        .expect("write workspace manifest");
        write_file(
            &project_root.join("mobench.toml"),
            br#"[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"

[android]
abis = ["arm64-v8a", "x86_64"]

[benchmarks]
default_function = "zk_mobile_bench::bench_query_proof_generation"

[browserstack]
ios_completion_timeout_secs = 900
"#,
        )
        .expect("write mobench config");
        write_file(
            &crate_dir.join("Cargo.toml"),
            br#"[package]
name = "zk-mobile-bench"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("write custom crate manifest");
        write_file(
            &crate_dir.join("src/lib.rs"),
            br#"#[benchmark]

pub fn bench_query_proof_generation() {}
"#,
        )
        .expect("write custom crate source");

        (
            project_root
                .canonicalize()
                .expect("canonicalize project root"),
            crate_dir.canonicalize().expect("canonicalize crate dir"),
        )
    }

    fn write_custom_layout_output_dir(project_root: &Path, output_dir: &Path) {
        write_file(
            &project_root.join("mobench.toml"),
            format!(
                r#"[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"
output_dir = {:?}
"#,
                output_dir.display().to_string()
            )
            .as_bytes(),
        )
        .expect("write output-dir layout config");
    }

    // Register a lightweight benchmark for tests so the inventory contains at least one entry.
    #[mobench_sdk::benchmark]
    fn noop_benchmark() {
        std::hint::black_box(1u8);
    }

    #[test]
    fn resolves_cli_spec() {
        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();
        let spec = resolve_run_spec(
            Some(MobileTarget::Android),
            Some("sample_fns::fibonacci".into()),
            Some(5),
            Some(1),
            vec!["pixel".into()],
            &layout,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            false, // release
            false,
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
    fn validate_spec_file_rejects_counts_larger_than_u32() {
        let out_of_range = u64::from(u32::MAX) + 1;

        for (field, value) in [
            (
                "iterations",
                json!({
                    "function": "sample_fns::fibonacci",
                    "iterations": out_of_range,
                    "warmup": 1
                }),
            ),
            (
                "warmup",
                json!({
                    "function": "sample_fns::fibonacci",
                    "iterations": 1,
                    "warmup": out_of_range
                }),
            ),
        ] {
            let temp_dir = TempDir::new().expect("temp dir");
            let spec_path = temp_dir.path().join("bench_spec.json");
            write_file(
                &spec_path,
                serde_json::to_vec(&value)
                    .expect("serialize oversized spec")
                    .as_slice(),
            )
            .expect("write oversized spec");

            let error = validate_spec_file(&spec_path).expect_err("oversized count must fail");
            assert!(
                error
                    .to_string()
                    .contains(&format!("spec.{field} must be an unsigned 32-bit integer")),
                "unexpected error for {field}: {error:#}"
            );
        }
    }

    #[test]
    fn validate_spec_file_requires_integral_bounded_counts() {
        for (field, invalid) in [
            ("iterations", json!(1.2)),
            ("iterations", json!(-1)),
            ("iterations", json!(MAX_BENCHMARK_COUNT + 1)),
            ("warmup", json!(1.2)),
            ("warmup", json!(-1)),
            ("warmup", json!(MAX_BENCHMARK_COUNT + 1)),
        ] {
            let temp_dir = TempDir::new().expect("temp dir");
            let spec_path = temp_dir.path().join("bench_spec.json");
            let mut value = json!({
                "function": "sample_fns::fibonacci",
                "iterations": 1,
                "warmup": 0
            });
            value[field] = invalid;
            write_file(
                &spec_path,
                serde_json::to_vec(&value)
                    .expect("serialize invalid spec")
                    .as_slice(),
            )
            .expect("write invalid spec");

            assert!(
                validate_spec_file(&spec_path).is_err(),
                "{field} should reject {}",
                value[field]
            );
        }
    }

    #[test]
    fn resolve_run_spec_prefers_cli_device_matrix_with_config() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_matrix_path = temp_dir.path().join("config-matrix.yml");
        let cli_matrix_path = temp_dir.path().join("cli-matrix.yml");
        let config_path = temp_dir.path().join("bench-config.toml");

        write_file(
            &config_matrix_path,
            br#"devices:
  - name: Config Device
    os: android
    os_version: "14"
"#,
        )
        .expect("write config matrix");
        write_file(
            &cli_matrix_path,
            br#"devices:
  - name: CLI Device
    os: android
    os_version: "14"
"#,
        )
        .expect("write cli matrix");

        let config_toml = format!(
            r#"target = "android"
function = "sample_fns::fibonacci"
iterations = 10
warmup = 2
device_matrix = "{}"

[browserstack]
app_automate_username = "user"
app_automate_access_key = "key"
project = "proj"
"#,
            config_matrix_path.display()
        );
        write_file(&config_path, config_toml.as_bytes()).expect("write config");

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();
        let spec = resolve_run_spec(
            Some(MobileTarget::Android),
            Some("ignored::value".into()),
            Some(1),
            Some(0),
            Vec::new(),
            &layout,
            Some(config_path.as_path()),
            Some(cli_matrix_path.as_path()),
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            false,
            false,
        )
        .expect("resolve spec");

        assert_eq!(spec.devices, vec!["CLI Device".to_string()]);
    }

    #[test]
    fn run_accepts_config_without_target_or_function_flags() {
        assert!(Cli::try_parse_from(["mobench", "run", "--config", "bench-config.toml"]).is_ok());
    }

    #[test]
    fn browserstack_collection_defaults_cover_observed_queue_latency() {
        let run = Cli::try_parse_from(["mobench", "run", "--config", "bench-config.toml"])
            .expect("parse run command");
        let Command::Run {
            fetch_timeout_secs, ..
        } = run.command
        else {
            panic!("expected run command");
        };
        assert_eq!(fetch_timeout_secs, 900);

        let ci = Cli::try_parse_from([
            "mobench",
            "ci",
            "run",
            "--target",
            "android",
            "--function",
            "bench",
        ])
        .expect("parse ci run command");
        let Command::Ci {
            command: CiCommand::Run(args),
        } = ci.command
        else {
            panic!("expected ci run command");
        };
        assert_eq!(args.fetch_timeout_secs, 900);
    }

    #[test]
    fn cli_rejects_invalid_execution_counts_before_dispatch() {
        let limit = MAX_BENCHMARK_COUNT.to_string();
        assert!(
            Cli::try_parse_from([
                "mobench",
                "run",
                "--target",
                "android",
                "--function",
                "bench",
                "--iterations",
                limit.as_str(),
                "--warmup",
                limit.as_str(),
            ])
            .is_ok()
        );

        for value in ["0", "1000001", "4294967295", "1.2", "-1"] {
            assert!(
                Cli::try_parse_from([
                    "mobench",
                    "run",
                    "--target",
                    "android",
                    "--function",
                    "bench",
                    "--iterations",
                    value,
                ])
                .is_err(),
                "iterations={value} should fail"
            );
        }

        for value in ["1000001", "4294967295", "1.2", "-1"] {
            assert!(
                Cli::try_parse_from([
                    "mobench",
                    "ci",
                    "run",
                    "--target",
                    "android",
                    "--function",
                    "bench",
                    "--warmup",
                    value,
                ])
                .is_err(),
                "warmup={value} should fail"
            );
        }
    }

    #[test]
    fn config_count_validation_uses_the_same_runtime_limit() {
        assert!(validate_run_counts(MAX_BENCHMARK_COUNT, MAX_BENCHMARK_COUNT).is_ok());
        assert!(validate_run_counts(0, 0).is_err());
        assert!(validate_run_counts(MAX_BENCHMARK_COUNT + 1, 0).is_err());
        assert!(validate_run_counts(1, MAX_BENCHMARK_COUNT + 1).is_err());
    }

    #[test]
    fn fixture_verify_plots_parses_fixture_name() {
        assert!(Cli::try_parse_from(["mobench", "fixture", "verify-plots", "basic",]).is_ok());
        assert!(
            Cli::try_parse_from([
                "mobench",
                "fixture",
                "verify-plots",
                "ffi",
                "--output-dir",
                "target/custom-fixture",
            ])
            .is_ok()
        );
    }

    #[test]
    fn resolve_run_spec_lets_cli_values_override_config_values() {
        let temp_dir = TempDir::new().expect("temp dir");
        let matrix_path = temp_dir.path().join("matrix.yml");
        let config_path = temp_dir.path().join("bench-config.toml");

        write_file(
            &matrix_path,
            br#"devices:
  - name: Config Device
    os: android
    os_version: "14"
"#,
        )
        .expect("write matrix");
        let config_toml = format!(
            r#"target = "android"
function = "config::function"
iterations = 10
warmup = 2
device_matrix = "{}"

[browserstack]
app_automate_username = "user"
app_automate_access_key = "key"
project = "proj"
"#,
            matrix_path.display()
        );
        write_file(&config_path, config_toml.as_bytes()).expect("write config");

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();
        let spec = resolve_run_spec(
            Some(MobileTarget::Ios),
            Some("cli::function".into()),
            Some(3),
            Some(1),
            Vec::new(),
            &layout,
            Some(config_path.as_path()),
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            false,
            false,
        )
        .expect("resolve spec");

        assert_eq!(spec.target, MobileTarget::Ios);
        assert_eq!(spec.function, "cli::function");
        assert_eq!(spec.iterations, 3);
        assert_eq!(spec.warmup, 1);
    }

    #[test]
    fn parses_project_resolution_flags() {
        assert!(
            Cli::try_parse_from([
                "mobench",
                "run",
                "--target",
                "ios",
                "--function",
                "zk_mobile_bench::bench_query_proof_generation",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "build",
                "--target",
                "ios",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "package-ipa",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "package-xcuitest",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "list",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
            ])
            .is_ok()
        );

        assert!(
            Cli::try_parse_from([
                "mobench",
                "verify",
                "--crate-path",
                "/tmp/custom-crate",
                "--project-root",
                "/tmp/project-root",
                "--smoke-test",
            ])
            .is_ok()
        );
    }

    #[test]
    fn resolver_uses_mobench_toml_for_custom_crate() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, crate_dir) = write_custom_layout_project(&temp_dir);

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");

        assert_eq!(layout.project_root, project_root);
        assert_eq!(layout.crate_dir, crate_dir);
        assert_eq!(layout.crate_name, "zk-mobile-bench");
        assert_eq!(layout.library_name, "zk_mobile_bench");
        assert_eq!(layout.ffi_backend, mobench_sdk::FfiBackend::Uniffi);
        assert_eq!(
            layout.android_abis,
            Some(vec!["arm64-v8a".to_string(), "x86_64".to_string()])
        );
        assert_eq!(layout.ios_completion_timeout_secs, Some(900));
        assert_eq!(layout.ios_deployment_target, "15.0");
        assert_eq!(layout.ios_runner, None);
        assert_eq!(layout.web_wasm_bindgen, None);
        assert_eq!(
            layout.default_function.as_deref(),
            Some("zk_mobile_bench::bench_query_proof_generation")
        );
    }

    #[test]
    fn resolver_reads_private_web_wasm_bindgen_extension() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);
        let config_path = project_root.join("mobench.toml");
        let mut contents = fs::read_to_string(&config_path).expect("read config");
        contents.push_str("\n[web]\nwasm_bindgen = \"tools/wasm-bindgen\"\n");
        write_file(&config_path, contents.as_bytes()).expect("write config");

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");

        assert_eq!(
            layout.web_wasm_bindgen,
            Some(PathBuf::from("tools/wasm-bindgen"))
        );
    }

    #[test]
    fn resolver_projects_configured_output_dir_beneath_project_without_creating_it() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);
        write_custom_layout_output_dir(&project_root, Path::new("artifacts/mobench"));

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve contained project layout");

        assert_eq!(layout.output_dir, project_root.join("artifacts/mobench"));
        assert!(!project_root.join("artifacts").exists());
    }

    #[test]
    fn resolver_rejects_absolute_configured_output_dir_before_writing() {
        let project_dir = TempDir::new().expect("project dir");
        let outside = TempDir::new().expect("outside dir");
        let (project_root, _) = write_custom_layout_project(&project_dir);
        let escaped = outside.path().join("mobench-output");
        write_custom_layout_output_dir(&project_root, &escaped);

        let error = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect_err("absolute configured output must be rejected");

        assert!(error.to_string().contains("project.output_dir"));
        assert!(!escaped.exists());
    }

    #[test]
    fn resolver_rejects_parent_traversal_configured_output_dir_before_writing() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);
        let escape_name = format!(
            "{}-escaped",
            project_root
                .file_name()
                .expect("project file name")
                .to_string_lossy()
        );
        let escaped = project_root
            .parent()
            .expect("project parent")
            .join(&escape_name);
        write_custom_layout_output_dir(&project_root, &Path::new("..").join(&escape_name));

        let error = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect_err("parent traversal output must be rejected");

        assert!(error.to_string().contains("project.output_dir"));
        assert!(!escaped.exists());
    }

    #[cfg(unix)]
    #[test]
    fn resolver_rejects_symlinked_configured_output_dir_before_writing() {
        use std::os::unix::fs::symlink;

        let project_dir = TempDir::new().expect("project dir");
        let outside = TempDir::new().expect("outside dir");
        let (project_root, _) = write_custom_layout_project(&project_dir);
        symlink(outside.path(), project_root.join("linked-output")).expect("create output symlink");
        write_custom_layout_output_dir(&project_root, Path::new("linked-output/mobench"));

        let error = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect_err("symlinked configured output must be rejected");

        assert!(error.to_string().contains("symbolic link"));
        assert!(!outside.path().join("mobench").exists());
    }

    #[cfg(unix)]
    #[test]
    fn resolver_rejects_symlinked_default_output_dir_before_writing() {
        use std::os::unix::fs::symlink;

        let project_dir = TempDir::new().expect("project dir");
        let outside = TempDir::new().expect("outside dir");
        let (project_root, _) = write_custom_layout_project(&project_dir);
        symlink(outside.path(), project_root.join("target")).expect("create target symlink");

        let error = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect_err("symlinked default output must be rejected");

        assert!(error.to_string().contains("symbolic link"));
        assert!(!outside.path().join("mobench").exists());
    }

    #[test]
    fn resolver_uses_typed_builder_configuration() {
        let cases = [
            ("uniffi", mobench_sdk::FfiBackend::Uniffi),
            ("native-c-abi", mobench_sdk::FfiBackend::NativeCAbi),
            ("boltffi", mobench_sdk::FfiBackend::BoltFfi),
        ];

        for (configured_backend, expected_backend) in cases {
            let temp_dir = TempDir::new().expect("temp dir");
            let (project_root, _) = write_custom_layout_project(&temp_dir);
            write_file(
                &project_root.join("mobench.toml"),
                format!(
                    r#"[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"
ffi_backend = "{configured_backend}"

[ios]
deployment_target = "14.0"
runner = "uikit-legacy"

[browserstack]
android_benchmark_timeout_secs = 120
android_heartbeat_interval_secs = 7
"#
                )
                .as_bytes(),
            )
            .expect("write mobench config");

            let layout = resolve_project_layout(ProjectLayoutOptions {
                start_dir: Some(project_root.as_path()),
                project_root: None,
                crate_path: None,
                config_path: None,
            })
            .expect("resolve project layout");

            assert_eq!(layout.ffi_backend, expected_backend);
            assert_eq!(layout.ios_runner.as_deref(), Some("uikit-legacy"));
            assert_eq!(layout.android_benchmark_timeout_secs, Some(120));
            assert_eq!(layout.android_heartbeat_interval_secs, Some(7));
        }
    }

    #[test]
    fn build_helpers_propagate_resolved_boltffi_backend() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, crate_dir) = write_custom_layout_project(&temp_dir);
        write_file(
            &project_root.join("mobench.toml"),
            br#"[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"
ffi_backend = "boltffi"
"#,
        )
        .expect("write mobench config");
        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");

        std::fs::remove_dir_all(crate_dir).expect("remove benchmark crate after resolution");

        let android_err = run_android_build(&layout, "", false, true)
            .expect_err("BoltFFI Android dry-run should inspect the configured crate");
        assert!(
            android_err
                .to_string()
                .contains("Specified crate path does not exist"),
            "unexpected Android error: {android_err}"
        );

        let ios_err = run_ios_build(&layout, false, true, None, None, None)
            .expect_err("BoltFFI iOS dry-run should inspect the configured crate");
        assert!(
            ios_err
                .to_string()
                .contains("Specified crate path does not exist"),
            "unexpected iOS error: {ios_err}"
        );
    }

    #[test]
    fn ios_runner_selection_uses_legacy_below_ios_15() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);
        write_file(
            &project_root.join("mobench.toml"),
            br#"[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"

[ios]
deployment_target = "10.0"

[benchmarks]
default_function = "zk_mobile_bench::bench_query_proof_generation"
"#,
        )
        .expect("write mobench config");

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");
        let target = configured_ios_deployment_target(&layout, None).unwrap();
        let runner = configured_ios_runner(&layout, &target, None).unwrap();

        assert_eq!(target.to_string(), "10.0");
        assert_eq!(runner, mobench_sdk::codegen::IosRunner::UikitLegacy);
    }

    #[test]
    fn ios_runner_rejects_forced_swiftui_below_ios_15() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);
        write_file(
            &project_root.join("mobench.toml"),
            br#"[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"

[ios]
deployment_target = "10.0"
runner = "swiftui"
"#,
        )
        .expect("write mobench config");

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");
        let target = configured_ios_deployment_target(&layout, None).unwrap();
        let err = configured_ios_runner(&layout, &target, None)
            .expect_err("swiftui should reject iOS 10");

        assert!(err.to_string().contains("requires deployment target 15.0+"));
    }

    #[test]
    fn list_uses_resolved_layout_for_custom_crate() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");

        let benchmarks = discover_benchmarks_for_layout(&layout).expect("discover benchmarks");
        assert_eq!(
            benchmarks,
            vec!["zk_mobile_bench::bench_query_proof_generation".to_string()]
        );
    }

    #[test]
    fn verify_external_crate_smoke_test_is_unsupported() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve project layout");

        let err = ensure_verify_smoke_test_supported(&layout)
            .expect_err("external crate smoke tests should be unsupported");
        assert!(
            err.to_string().contains("external crate"),
            "unexpected error: {err}"
        );
        assert!(
            err.to_string().contains("unsupported"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn build_progress_uses_configured_crate() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        cmd_build(
            SdkTarget::Ios,
            false,
            None,
            None,
            None,
            Some(project_root),
            None,
            None,
            true,
            false,
            true,
        )
        .expect("build --progress should resolve config-driven crate");
    }

    #[test]
    fn verify_smoke_test_skips_external_crate() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        cmd_verify(
            Some(project_root),
            None,
            None,
            None,
            false,
            true,
            Some("zk_mobile_bench::bench_query_proof_generation".to_string()),
            None,
        )
        .expect("verify should clearly skip unsupported external smoke tests");
    }

    #[test]
    fn run_dry_run_prepares_ios_artifacts_inside_custom_project() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve layout");
        let spec = resolve_run_spec(
            Some(MobileTarget::Ios),
            Some("zk_mobile_bench::bench_query_proof_generation".into()),
            Some(1),
            Some(0),
            vec!["iPhone 15".into()],
            &layout,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            false,
            true,
        )
        .expect("resolve dry-run spec");

        let ios_xcuitest = spec
            .ios_xcuitest
            .expect("dry-run should prepare placeholder iOS artifacts");
        assert_eq!(spec.ios_completion_timeout_secs, Some(900));
        assert!(
            ios_xcuitest.app.starts_with(&project_root),
            "app path should stay inside project root: {}",
            ios_xcuitest.app.display()
        );
        assert!(
            ios_xcuitest.test_suite.starts_with(&project_root),
            "test suite path should stay inside project root: {}",
            ios_xcuitest.test_suite.display()
        );
        assert!(
            ios_xcuitest
                .app
                .ends_with(Path::new("target/mobench/ios/BenchRunner.ipa"))
        );
        assert!(
            ios_xcuitest
                .test_suite
                .ends_with(Path::new("target/mobench/ios/BenchRunnerUITests.zip"))
        );
    }

    #[test]
    fn snapshot_baseline_creates_distinct_copy() {
        let temp_dir = TempDir::new().expect("temp dir");
        let baseline = temp_dir.path().join("baseline.json");
        write_file(&baseline, br#"{"ok":true}"#).expect("write baseline");

        assert!(paths_point_to_same_file(&baseline, &baseline).expect("compare path"));

        let snapshot = snapshot_baseline_for_compare(&baseline).expect("snapshot baseline");
        assert_ne!(snapshot, baseline);
        let original_contents = fs::read_to_string(&baseline).expect("read baseline");
        let snapshot_contents = fs::read_to_string(&snapshot).expect("read snapshot");
        assert_eq!(snapshot_contents, original_contents);

        fs::remove_file(snapshot).expect("remove snapshot");
    }

    #[test]
    fn local_smoke_produces_samples() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "noop_benchmark".into(),
            iterations: 3,
            warmup: 1,
            devices: vec![],
            ios_completion_timeout_secs: None,
            ios_deployment_target: None,
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
            browserstack: None,
            ios_xcuitest: None,
        };
        let report = run_local_smoke(&spec).expect("local harness");
        assert!(report["samples"].is_array());
        assert_eq!(report["spec"]["name"], "noop_benchmark");
    }

    #[test]
    fn embedded_mobile_spec_carries_v2_logical_identity() {
        let temp_dir = TempDir::new().expect("temp dir");
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            iterations: 3,
            warmup: 1,
            devices: vec!["Google Pixel 7-13.0".into()],
            ios_completion_timeout_secs: None,
            ios_deployment_target: None,
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
            browserstack: None,
            ios_xcuitest: None,
        };
        let identity = RunEnvelopeIdentity {
            run_id: "run-test-001".into(),
            nonce: "nonce-test-001".into(),
            logical_session_id: "logical-session-test-001".into(),
            producer: "android-runner".into(),
        };

        embed_spec_into_apps(temp_dir.path(), &spec, &identity).expect("embed mobile spec");
        let value: Value = serde_json::from_slice(
            &fs::read(
                temp_dir
                    .path()
                    .join("target/mobile-spec/android/bench_spec.json"),
            )
            .expect("read embedded Android spec"),
        )
        .expect("parse embedded spec");

        assert_eq!(value["schema_version"], mobench_domain::REPORT_SCHEMA_V2);
        assert_eq!(value["run_id"], "run-test-001");
        assert_eq!(value["nonce"], "nonce-test-001");
        assert_eq!(value["logical_session_id"], "logical-session-test-001");
        assert_eq!(value["function_id"], "sample_fns::fibonacci");
        assert_eq!(value["producer"], "android-runner");
    }

    #[test]
    fn ios_defers_packaging_browserstack_artifacts_until_run_time() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);
        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve layout");
        let spec = resolve_run_spec(
            Some(MobileTarget::Ios),
            Some("zk_mobile_bench::bench_query_proof_generation".into()),
            Some(1),
            Some(0),
            vec!["iPhone 15".into()],
            &layout,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            false, // release
            false,
        )
        .expect("should prepare iOS BrowserStack artifact paths");
        let ios_artifacts = spec
            .ios_xcuitest
            .expect("iOS artifact paths should be populated");
        assert_eq!(
            ios_artifacts.app,
            layout.output_dir.join("ios/BenchRunner.ipa")
        );
        assert!(
            ios_artifacts
                .test_suite
                .ends_with(Path::new("target/mobench/ios/BenchRunnerUITests.zip"))
        );
        assert!(
            !ios_artifacts.app.exists(),
            "iOS app artifact should not be packaged before the current bench_spec is persisted"
        );
        assert!(
            !ios_artifacts.test_suite.exists(),
            "iOS test suite should not be packaged before the current bench_spec is persisted"
        );
    }

    #[test]
    fn ios_managed_artifact_detection_accepts_config_template_paths() {
        let temp_dir = TempDir::new().expect("temp dir");
        let (project_root, _) = write_custom_layout_project(&temp_dir);
        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: Some(project_root.as_path()),
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .expect("resolve layout");

        let config_template_artifacts = IosXcuitestArtifacts {
            app: PathBuf::from("target/ios/BenchRunner.ipa"),
            test_suite: PathBuf::from("target/ios/BenchRunnerUITests.zip"),
        };

        assert!(
            uses_managed_ios_xcuitest_artifacts(&layout, &config_template_artifacts),
            "legacy config template paths should still be treated as mobench-managed artifacts"
        );
    }

    #[test]
    fn format_duration_smart_uses_milliseconds_by_default() {
        // 500 microseconds = 0.5 ms
        assert_eq!(format_duration_smart(500_000), "0.500ms");
        // 1.5 ms
        assert_eq!(format_duration_smart(1_500_000), "1.500ms");
        // 100 ms
        assert_eq!(format_duration_smart(100_000_000), "100.000ms");
        // 999.999 ms (just below threshold)
        assert_eq!(format_duration_smart(999_999_000), "999.999ms");
    }

    #[test]
    fn format_duration_smart_switches_to_seconds_when_large() {
        // Exactly 1 second
        assert_eq!(format_duration_smart(1_000_000_000), "1.000s");
        // 1.5 seconds
        assert_eq!(format_duration_smart(1_500_000_000), "1.500s");
        // 10 seconds
        assert_eq!(format_duration_smart(10_000_000_000), "10.000s");
    }

    #[test]
    fn format_ms_handles_optional_values() {
        assert_eq!(format_ms(Some(1_500_000)), "1.500ms");
        assert_eq!(format_ms(Some(1_500_000_000)), "1.500s");
        assert_eq!(format_ms(None), "-");
    }

    #[test]
    fn doctor_browserstack_defaults_to_true() {
        let cli = Cli::parse_from(["mobench", "doctor"]);
        match cli.command {
            Command::Doctor { browserstack, .. } => assert!(browserstack),
            _ => panic!("expected doctor command"),
        }
    }

    #[test]
    fn doctor_browserstack_can_be_disabled() {
        let cli = Cli::parse_from(["mobench", "doctor", "--browserstack=false"]);
        match cli.command {
            Command::Doctor { browserstack, .. } => assert!(!browserstack),
            _ => panic!("expected doctor command"),
        }
    }

    #[test]
    fn doctor_android_prereqs_default_to_arm64_only() {
        assert_eq!(
            DEFAULT_ANDROID_DOCTOR_RUST_TARGETS,
            &["aarch64-linux-android"]
        );
    }

    #[test]
    fn rustc_msrv_parser_handles_stable_and_prerelease_versions() {
        assert_eq!(
            parse_rust_version("rustc 1.95.0 (59807616e 2026-04-14)"),
            Some((1, 95, 0))
        );
        assert_eq!(
            parse_rust_version("rustc 1.85.0-beta.1 (example)"),
            Some((1, 85, 0))
        );
        assert!(rustc_version_meets_msrv("rustc 1.85.0", WORKSPACE_MSRV));
        assert!(rustc_version_meets_msrv("rustc 1.95.0", WORKSPACE_MSRV));
        assert!(!rustc_version_meets_msrv("rustc 1.84.1", WORKSPACE_MSRV));
    }

    #[test]
    fn ci_run_parses_required_args_with_defaults() {
        let cli = Cli::parse_from([
            "mobench",
            "ci",
            "run",
            "--target",
            "android",
            "--function",
            "sample_fns::fibonacci",
        ]);

        match cli.command {
            Command::Ci {
                command: CiCommand::Run(args),
            } => {
                assert_eq!(args.target, CiTarget::Android);
                assert_eq!(args.function.as_deref(), Some("sample_fns::fibonacci"));
                assert_eq!(args.output_dir, PathBuf::from("target/mobench/ci"));
            }
            _ => panic!("expected ci run command"),
        }
    }

    #[test]
    fn ci_run_parses_both_target() {
        let cli = Cli::parse_from([
            "mobench",
            "ci",
            "run",
            "--target",
            "both",
            "--function",
            "sample_fns::fibonacci",
        ]);

        match cli.command {
            Command::Ci {
                command: CiCommand::Run(args),
            } => {
                assert_eq!(args.target, CiTarget::Both);
            }
            _ => panic!("expected ci run command"),
        }
    }

    #[test]
    fn ci_run_parses_ios_completion_timeout_secs() {
        let cli = Cli::parse_from([
            "mobench",
            "ci",
            "run",
            "--target",
            "ios",
            "--function",
            "sample_fns::fibonacci",
            "--ios-completion-timeout-secs",
            "900",
        ]);

        match cli.command {
            Command::Ci {
                command: CiCommand::Run(args),
            } => {
                assert_eq!(args.target, CiTarget::Ios);
                assert_eq!(args.ios_completion_timeout_secs, Some(900));
            }
            _ => panic!("expected ci run command"),
        }
    }

    #[test]
    fn ci_run_parses_android_watchdog_settings() {
        let cli = Cli::parse_from([
            "mobench",
            "ci",
            "run",
            "--target",
            "android",
            "--function",
            "sample_fns::fibonacci",
            "--android-benchmark-timeout-secs",
            "30",
            "--android-heartbeat-interval-secs",
            "3",
        ]);

        match cli.command {
            Command::Ci {
                command: CiCommand::Run(args),
            } => {
                assert_eq!(args.target, CiTarget::Android);
                assert_eq!(args.android_benchmark_timeout_secs, Some(30));
                assert_eq!(args.android_heartbeat_interval_secs, Some(3));
            }
            _ => panic!("expected ci run command"),
        }
    }

    #[test]
    fn build_parses_ios_completion_timeout_secs() {
        let cli = Cli::parse_from([
            "mobench",
            "build",
            "--target",
            "ios",
            "--ios-completion-timeout-secs",
            "750",
        ]);

        match cli.command {
            Command::Build {
                ios_completion_timeout_secs,
                ..
            } => {
                assert_eq!(ios_completion_timeout_secs, Some(750));
            }
            _ => panic!("expected build command"),
        }
    }

    #[test]
    fn build_parses_ios_deployment_target_and_runner() {
        let cli = Cli::parse_from([
            "mobench",
            "build",
            "--target",
            "ios",
            "--ios-deployment-target",
            "10.0",
            "--ios-runner",
            "uikit-legacy",
        ]);

        match cli.command {
            Command::Build {
                ios_deployment_target,
                ios_runner,
                ..
            } => {
                assert_eq!(ios_deployment_target.as_deref(), Some("10.0"));
                assert_eq!(ios_runner, Some(IosRunnerArg::UikitLegacy));
            }
            _ => panic!("expected build command"),
        }
    }

    #[test]
    fn build_parses_web_without_expanding_public_sdk_target() {
        let cli = Cli::parse_from(["mobench", "build", "--target", "web", "--release"]);
        match cli.command {
            Command::Build {
                target, release, ..
            } => {
                assert_eq!(target, BuildTarget::Web);
                assert!(release);
                assert_eq!(target.mobile(), None);
            }
            _ => panic!("expected web build command"),
        }
    }

    #[test]
    fn run_web_parses_mobile_local_environment() {
        let cli = Cli::parse_from([
            "mobench",
            "run-web",
            "--url",
            "https://bench.example.test/",
            "--function",
            "sample::bench",
            "--device",
            "iPhone 16 Pro Max",
            "--os-version",
            "18",
            "--local-identifier",
            "mobench-run-1",
        ]);
        match cli.command {
            Command::RunWeb {
                url,
                function,
                device,
                os_version,
                local_identifier,
                ..
            } => {
                assert_eq!(url, "https://bench.example.test/");
                assert_eq!(function, "sample::bench");
                assert_eq!(device.as_deref(), Some("iPhone 16 Pro Max"));
                assert_eq!(os_version, "18");
                assert_eq!(local_identifier.as_deref(), Some("mobench-run-1"));
            }
            _ => panic!("expected run-web command"),
        }
    }

    #[test]
    fn ci_prebuilt_commands_parse_trusted_controls() {
        let prepare = Cli::parse_from([
            "mobench",
            "ci",
            "prepare",
            "--target",
            "android",
            "--functions",
            "sample::bench",
            "--source-sha",
            "0123456789abcdef0123456789abcdef01234567",
        ]);
        assert!(matches!(
            prepare.command,
            Command::Ci {
                command: CiCommand::Prepare(_)
            }
        ));

        let run = Cli::parse_from([
            "mobench",
            "ci",
            "run-prebuilt",
            "--manifest",
            "bundle/manifest.json",
            "--expected-source-sha",
            "0123456789abcdef0123456789abcdef01234567",
            "--expected-platform",
            "android",
            "--expected-functions",
            "sample::bench",
            "--expected-iterations",
            "2",
            "--expected-warmup",
            "1",
            "--devices",
            "Google Pixel 7-13.0",
        ]);
        match run.command {
            Command::Ci {
                command: CiCommand::RunPrebuilt(args),
            } => {
                assert_eq!(args.expected_iterations, 2);
                assert_eq!(args.max_completion_timeout_secs, 1800);
            }
            _ => panic!("expected ci run-prebuilt command"),
        }
    }

    #[test]
    fn resolve_run_spec_reads_ios_completion_timeout_from_config() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("bench-config.toml");

        let config_toml = r#"target = "ios"
function = "sample_fns::fibonacci"
iterations = 10
warmup = 2
device_matrix = "device-matrix.yaml"

[browserstack]
app_automate_username = "user"
app_automate_access_key = "key"
project = "proj"
ios_completion_timeout_secs = 900

[ios_xcuitest]
app = "target/ios/BenchRunner.ipa"
test_suite = "target/ios/BenchRunnerUITests.zip"
"#;
        write_file(&config_path, config_toml.as_bytes()).expect("write config");
        write_file(
            &temp_dir.path().join("device-matrix.yaml"),
            br#"devices:
  - name: iPhone 16 Pro
    os: ios
    os_version: "18"
"#,
        )
        .expect("write matrix");

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();
        let spec = resolve_run_spec(
            Some(MobileTarget::Ios),
            Some("ignored::value".into()),
            Some(1),
            Some(0),
            Vec::new(),
            &layout,
            Some(config_path.as_path()),
            None,
            Vec::new(),
            None,
            None,
            Some(600),
            None,
            None,
            None,
            None,
            false,
            false,
            false,
        )
        .expect("resolve spec");

        assert_eq!(spec.ios_completion_timeout_secs, Some(600));
        assert_eq!(spec.ios_deployment_target.as_deref(), Some("15.0"));
        assert_eq!(spec.ios_runner.as_deref(), Some("swiftui"));
        assert_eq!(
            spec.browserstack
                .as_ref()
                .and_then(|cfg| cfg.ios_completion_timeout_secs),
            Some(900)
        );
    }

    #[test]
    fn resolve_run_spec_applies_legacy_ios_deployment_override() {
        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();

        let spec = resolve_run_spec(
            Some(MobileTarget::Ios),
            Some("sample_fns::fibonacci".into()),
            Some(1),
            Some(0),
            vec!["iPhone 7-10".to_string()],
            &layout,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            Some("10.0".to_string()),
            None,
            None,
            None,
            false,
            false,
            false,
        )
        .expect("resolve spec");

        assert_eq!(spec.ios_deployment_target.as_deref(), Some("10.0"));
        assert_eq!(spec.ios_runner.as_deref(), Some("uikit-legacy"));
    }

    #[test]
    fn resolve_run_spec_rejects_ios_device_below_deployment_target() {
        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();

        let err = resolve_run_spec(
            Some(MobileTarget::Ios),
            Some("sample_fns::fibonacci".into()),
            Some(1),
            Some(0),
            vec!["iPhone 7-10".to_string()],
            &layout,
            None,
            None,
            Vec::new(),
            None,
            None,
            None,
            Some("15.0".to_string()),
            None,
            None,
            None,
            false,
            false,
            false,
        )
        .expect_err("iOS 10 device should reject iOS 15 app");

        assert!(
            err.to_string()
                .contains("cannot run app with iOS deployment target `15.0`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_run_spec_reads_android_watchdog_from_config_and_cli() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("bench-config.toml");

        let config_toml = r#"target = "android"
function = "sample_fns::fibonacci"
iterations = 10
warmup = 2
device_matrix = "device-matrix.yaml"

[browserstack]
app_automate_username = "user"
app_automate_access_key = "key"
project = "proj"
android_benchmark_timeout_secs = 120
android_heartbeat_interval_secs = 7
"#;
        write_file(&config_path, config_toml.as_bytes()).expect("write config");
        write_file(
            &temp_dir.path().join("device-matrix.yaml"),
            br#"devices:
  - name: Google Pixel 8
    os: android
    os_version: "14"
"#,
        )
        .expect("write matrix");

        let layout = resolve_project_layout(ProjectLayoutOptions {
            start_dir: None,
            project_root: None,
            crate_path: None,
            config_path: None,
        })
        .unwrap();
        let spec = resolve_run_spec(
            Some(MobileTarget::Android),
            Some("ignored::value".into()),
            Some(1),
            Some(0),
            Vec::new(),
            &layout,
            Some(config_path.as_path()),
            None,
            Vec::new(),
            None,
            None,
            None,
            None,
            None,
            Some(30),
            Some(3),
            false,
            false,
            false,
        )
        .expect("resolve spec");

        assert_eq!(spec.android_benchmark_timeout_secs, Some(30));
        assert_eq!(spec.android_heartbeat_interval_secs, Some(3));
    }

    #[test]
    fn devices_resolve_parses() {
        let cli = Cli::parse_from([
            "mobench",
            "devices",
            "resolve",
            "--platform",
            "android",
            "--profile",
            "default",
            "--device-matrix",
            "device-matrix.yaml",
        ]);
        match cli.command {
            Command::Devices {
                command:
                    Some(DevicesCommand::Resolve {
                        platform, profile, ..
                    }),
                ..
            } => {
                assert_eq!(platform, DevicePlatform::Android);
                assert_eq!(profile, Some("default".to_string()));
            }
            _ => panic!("expected devices resolve command"),
        }
    }

    #[test]
    fn fixture_cache_key_parses() {
        let cli = Cli::parse_from(["mobench", "fixture", "cache-key"]);
        match cli.command {
            Command::Fixture {
                command:
                    FixtureCommand::CacheKey {
                        config,
                        target,
                        format,
                        ..
                    },
            } => {
                assert_eq!(config, PathBuf::from("bench-config.toml"));
                assert_eq!(target, SdkTarget::Both);
                assert_eq!(format, CheckOutputFormat::Text);
            }
            _ => panic!("expected fixture cache-key command"),
        }
    }

    #[test]
    fn profile_run_parses_with_android_backend() {
        let cli = Cli::parse_from([
            "mobench",
            "profile",
            "run",
            "--target",
            "android",
            "--function",
            "sample_fns::fibonacci",
            "--backend",
            "android-native",
        ]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Run(args),
            } => {
                assert_eq!(args.target, MobileTarget::Android);
                assert_eq!(args.function, "sample_fns::fibonacci");
                assert_eq!(args.backend, profile::ProfileBackend::AndroidNative);
            }
            _ => panic!("expected profile run command"),
        }
    }

    #[test]
    fn profile_run_parses_direct_device_selection() {
        let cli = Cli::parse_from([
            "mobench",
            "profile",
            "run",
            "--target",
            "ios",
            "--function",
            "sample_fns::fibonacci",
            "--provider",
            "browserstack",
            "--backend",
            "ios-instruments",
            "--device",
            "iPhone 14",
            "--os-version",
            "16",
        ]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Run(args),
            } => {
                assert_eq!(args.target, MobileTarget::Ios);
                assert_eq!(args.device.as_deref(), Some("iPhone 14"));
                assert_eq!(args.os_version.as_deref(), Some("16"));
            }
            _ => panic!("expected profile run command"),
        }
    }

    #[test]
    fn profile_run_parses_profile_device_resolution_inputs() {
        let cli = Cli::parse_from([
            "mobench",
            "profile",
            "run",
            "--target",
            "ios",
            "--function",
            "sample_fns::fibonacci",
            "--provider",
            "browserstack",
            "--backend",
            "ios-instruments",
            "--profile",
            "high-spec",
            "--device-matrix",
            "device-matrix.yaml",
        ]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Run(args),
            } => {
                assert_eq!(args.profile.as_deref(), Some("high-spec"));
                assert_eq!(
                    args.device_matrix,
                    Some(PathBuf::from("device-matrix.yaml"))
                );
            }
            _ => panic!("expected profile run command"),
        }
    }

    #[test]
    fn profile_run_parses_capture_warmup_mode() {
        let cli = Cli::parse_from([
            "mobench",
            "profile",
            "run",
            "--target",
            "android",
            "--function",
            "sample_fns::fibonacci",
            "--warmup-mode",
            "cold",
        ]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Run(args),
            } => {
                assert_eq!(args.warmup_mode, Some(profile::CaptureWarmupMode::Cold));
            }
            _ => panic!("expected profile run command"),
        }
    }

    #[test]
    fn profile_run_help_mentions_planned_only_or_execution_scope() {
        let help = render_profile_run_help();

        assert!(
            help.contains("Plan or execute a native profiling session; local android-native and ios-instruments now attempt real native capture"),
            "expected profile run help to describe the real local Android/iOS execution scope, got:\n{help}"
        );
        assert!(
            help.contains(
                "local + android-native: attempts real simpleperf capture and symbolization"
            ),
            "expected profile run help to mention real Android native execution, got:\n{help}"
        );
        assert!(
            help.contains(
                "local + ios-instruments: attempts real simulator-host sample capture and flamegraph generation"
            ),
            "expected profile run help to mention real local iOS sample capture, got:\n{help}"
        );
        assert!(
            help.contains("--warmup-mode"),
            "expected profile run help to expose warm/cold profiling mode, got:\n{help}"
        );
    }

    #[test]
    fn profile_run_cli_surface_exposes_or_explicitly_omits_device_selection() {
        let help = render_profile_run_help();

        assert!(
            help.contains("--device")
                || help.contains("--profile")
                || help.contains("--device-matrix")
                || help.contains("device selection is unavailable"),
            "expected profile run help to either expose device selection or explicitly document that it is unavailable, got:\n{help}"
        );
    }

    #[test]
    fn profile_summarize_parses_with_default_profile_path() {
        let cli = Cli::parse_from(["mobench", "profile", "summarize"]);

        match cli.command {
            Command::Profile {
                command: ProfileCommand::Summarize(args),
            } => {
                assert_eq!(
                    args.profile,
                    PathBuf::from("target/mobench/profile/profile.json")
                );
                assert_eq!(args.output_format, profile::ProfileSummaryFormat::Markdown);
            }
            _ => panic!("expected profile summarize command"),
        }
    }

    #[test]
    fn report_github_parses() {
        let cli = Cli::parse_from(["mobench", "report", "github", "--pr", "123"]);
        match cli.command {
            Command::Report {
                command: ReportCommand::Github { pr, publish, .. },
            } => {
                assert_eq!(pr, Some("123".to_string()));
                assert!(!publish);
            }
            _ => panic!("expected report github command"),
        }
    }

    #[test]
    fn config_validate_parses_required_args_with_defaults() {
        let cli = Cli::parse_from(["mobench", "config", "validate"]);
        match cli.command {
            Command::Config {
                command: ConfigCommand::Validate { config, format },
            } => {
                assert_eq!(config, PathBuf::from("bench-config.toml"));
                assert_eq!(format, CheckOutputFormat::Text);
            }
            _ => panic!("expected config validate command"),
        }
    }

    #[test]
    fn issue_categories_align_with_contract_taxonomy() {
        let checks = vec![
            PrereqCheck {
                name: "Run config".to_string(),
                passed: false,
                detail: Some("missing".to_string()),
                fix_hint: Some("fix config".to_string()),
            },
            PrereqCheck {
                name: "BrowserStack credentials".to_string(),
                passed: false,
                detail: Some("missing".to_string()),
                fix_hint: Some("set env".to_string()),
            },
            PrereqCheck {
                name: "cargo installed".to_string(),
                passed: false,
                detail: None,
                fix_hint: Some("install rust".to_string()),
            },
        ];
        let issues = collect_issues(&checks);
        assert_eq!(issues.len(), 3);
        assert_eq!(category_slug(issues[0].category), "config_error");
        assert_eq!(category_slug(issues[1].category), "provider_error");
        assert_eq!(category_slug(issues[2].category), "preflight_error");
    }

    #[test]
    fn check_results_json_includes_issue_categories() {
        let checks = vec![PrereqCheck {
            name: "Run config".to_string(),
            passed: false,
            detail: Some("missing".to_string()),
            fix_hint: Some("fix config".to_string()),
        }];
        let issues = collect_issues(&checks);
        let rendered = render_check_results_json(&checks, &issues);
        let category = rendered
            .get("issues")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|first| first.get("category"))
            .and_then(|v| v.as_str());
        assert_eq!(category, Some("config"));
    }

    #[test]
    fn resolve_devices_from_matrix_is_deterministic() {
        let devices = vec![
            DeviceEntry {
                name: "Pixel 7".to_string(),
                os: "android".to_string(),
                os_version: "13.0".to_string(),
                tags: Some(vec!["default".to_string(), "pixel".to_string()]),
            },
            DeviceEntry {
                name: "Pixel 6".to_string(),
                os: "android".to_string(),
                os_version: "12.0".to_string(),
                tags: Some(vec!["default".to_string()]),
            },
            DeviceEntry {
                name: "iPhone 14".to_string(),
                os: "ios".to_string(),
                os_version: "16".to_string(),
                tags: Some(vec!["default".to_string(), "iphone".to_string()]),
            },
        ];

        let resolved =
            resolve_devices_from_matrix(devices, DevicePlatform::Android, &["default".to_string()])
                .expect("resolved devices");
        let ids: Vec<String> = resolved.into_iter().map(|d| d.identifier).collect();
        assert_eq!(ids, vec!["Pixel 6-12.0", "Pixel 7-13.0"]);
    }

    #[test]
    fn browserstack_identifier_preserves_versioned_ios_names() {
        let (identifier, os_version) = browserstack_identifier_and_os_version("iPhone 7-10", "");
        assert_eq!(identifier, "iPhone 7-10");
        assert_eq!(os_version, "10");

        let (identifier, os_version) =
            browserstack_identifier_and_os_version("iPhone 7-10", "10.0");
        assert_eq!(identifier, "iPhone 7-10");
        assert_eq!(os_version, "10.0");
    }

    fn safe_config_string() -> impl Strategy<Value = String> {
        "[A-Za-z0-9_. -]{1,32}".prop_map(|s| s.trim().to_string())
    }

    fn safe_path_string() -> impl Strategy<Value = PathBuf> {
        "[a-z0-9_/-]{1,32}".prop_map(PathBuf::from)
    }

    fn generated_bench_config() -> impl Strategy<Value = BenchConfig> {
        (
            prop_oneof![Just(MobileTarget::Android), Just(MobileTarget::Ios)],
            safe_config_string(),
            1_u32..10_000,
            0_u32..1_000,
            safe_path_string(),
            prop::collection::vec(safe_config_string(), 0..5),
            safe_config_string(),
            safe_config_string(),
            prop::option::of(safe_config_string()),
        )
            .prop_map(
                |(
                    target,
                    function,
                    iterations,
                    warmup,
                    device_matrix,
                    device_tags,
                    username,
                    access_key,
                    project,
                )| BenchConfig {
                    target,
                    function,
                    iterations,
                    warmup,
                    device_matrix,
                    device_tags: Some(device_tags).filter(|tags| !tags.is_empty()),
                    browserstack: BrowserStackConfig {
                        app_automate_username: username,
                        app_automate_access_key: access_key,
                        project,
                        ios_completion_timeout_secs: None,
                        android_benchmark_timeout_secs: None,
                        android_heartbeat_interval_secs: None,
                    },
                    ios_xcuitest: None,
                },
            )
    }

    fn generated_device_entry() -> impl Strategy<Value = DeviceEntry> {
        (
            safe_config_string(),
            prop_oneof![Just("android".to_string()), Just("ios".to_string())],
            "[0-9.]{1,8}",
            prop::collection::vec(safe_config_string(), 0..5),
        )
            .prop_map(|(name, os, os_version, tags)| DeviceEntry {
                name,
                os,
                os_version,
                tags: Some(tags).filter(|tags| !tags.is_empty()),
            })
    }

    proptest! {
        #[test]
        fn generated_valid_run_configs_parse(config in generated_bench_config()) {
            let encoded = toml::to_string(&config).expect("serialize generated run config");
            let parsed: BenchConfig = toml::from_str(&encoded).expect("parse generated run config");

            prop_assert_eq!(parsed.target, config.target);
            prop_assert_eq!(parsed.function, config.function);
            prop_assert_eq!(parsed.iterations, config.iterations);
            prop_assert_eq!(parsed.warmup, config.warmup);
            prop_assert_eq!(parsed.device_matrix, config.device_matrix);
        }

        #[test]
        fn generated_valid_device_matrices_parse(
            devices in prop::collection::vec(generated_device_entry(), 0..20)
        ) {
            let matrix = DeviceMatrix { devices };
            let encoded = serde_yaml::to_string(&matrix).expect("serialize generated device matrix");
            let parsed: DeviceMatrix = serde_yaml::from_str(&encoded).expect("parse generated device matrix");

            prop_assert_eq!(parsed.devices.len(), matrix.devices.len());
        }
    }

    #[test]
    fn builtin_ios_low_spec_profile_uses_iphone_se_2020() {
        let resolved = builtin_device_for_profile(DevicePlatform::Ios, "low-spec")
            .expect("built-in low-spec iOS profile");

        assert_eq!(resolved.name, "iPhone SE 2020");
        assert_eq!(resolved.os_version, "16");
        assert_eq!(resolved.identifier, "iPhone SE 2020-16");
    }

    #[test]
    fn builtin_android_low_spec_profile_uses_moto_g9_play() {
        let resolved = builtin_device_for_profile(DevicePlatform::Android, "low-spec")
            .expect("built-in low-spec Android profile");

        assert_eq!(resolved.name, "Motorola Moto G9 Play");
        assert_eq!(resolved.os_version, "10.0");
        assert_eq!(resolved.identifier, "Motorola Moto G9 Play-10.0");
    }

    #[test]
    fn render_summary_markdown_from_merged_output() {
        let summary = json!({
            "generated_at": "2026-02-16T00:00:00Z",
            "generated_at_unix": 1708041600,
            "target": "android",
            "function": "noop_benchmark",
            "iterations": 3,
            "warmup": 1,
            "devices": ["local"],
            "device_summaries": []
        });
        let merged = json!({
            "targets": {
                "android": { "summary": summary },
                "ios": { "summary": {
                    "generated_at": "2026-02-16T00:00:00Z",
                    "generated_at_unix": 1708041600,
                    "target": "ios",
                    "function": "noop_benchmark",
                    "iterations": 3,
                    "warmup": 1,
                    "devices": ["local"],
                    "device_summaries": []
                }}
            }
        });
        let markdown = render_summary_markdown_from_output(&merged).expect("render markdown");
        assert!(markdown.contains("## android"));
        assert!(markdown.contains("## ios"));
    }

    #[test]
    fn compare_markdown_includes_delta_labels() {
        let report = CompareReport {
            baseline: PathBuf::from("baseline.json"),
            candidate: PathBuf::from("candidate.json"),
            rows: vec![CompareRow {
                device: "Pixel 7".to_string(),
                function: "noop_benchmark".to_string(),
                baseline_median_ns: Some(100),
                candidate_median_ns: Some(110),
                median_delta_pct: Some(10.0),
                median_label: "regressed".to_string(),
                baseline_p95_ns: Some(120),
                candidate_p95_ns: Some(118),
                p95_delta_pct: Some(-1.66),
                p95_label: "improved".to_string(),
            }],
        };
        let markdown = render_compare_markdown(&report);
        assert!(markdown.starts_with("### Benchmark Comparison\n"));
        assert!(markdown.contains("Median base"));
        assert!(markdown.contains("Median cand"));
        assert!(markdown.contains("P95 base"));
        assert!(markdown.contains("P95 cand"));
        assert!(!markdown.contains("Median (base ms)"));
        assert!(!markdown.contains("Median (cand ms)"));
        assert!(!markdown.contains("P95 (base ms)"));
        assert!(!markdown.contains("P95 (cand ms)"));
        assert!(markdown.contains("Median Label"));
        assert!(markdown.contains("P95 Label"));
        assert!(markdown.contains("regressed"));
        assert!(markdown.contains("improved"));
    }

    #[test]
    fn render_markdown_summary_includes_resource_usage_columns_when_present() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(249_416),
                        peak_memory_growth_kb: Some(249_416),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                    failure: None,
                }],
            }],
        });

        assert!(markdown.contains(
            "| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |"
        ));
        assert!(markdown.contains("1.250s"));
        assert!(markdown.contains("6.250s"));
        assert!(markdown.contains("241ms"));
        assert!(markdown.contains("482ms"));
        assert!(markdown.contains("7.7%"));
        assert!(markdown.contains("243.57 MB"));
    }

    #[test]
    fn render_markdown_summary_uses_explicit_wall_and_cpu_columns() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 4,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 4,
                    mean_ns: Some(1_000_000_000),
                    median_ns: Some(950_000_000),
                    p95_ns: Some(1_100_000_000),
                    min_ns: Some(900_000_000),
                    max_ns: Some(1_200_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(800),
                        cpu_median_ms: Some(200),
                        peak_memory_kb: Some(1_024),
                        peak_memory_growth_kb: Some(1_024),
                        process_peak_memory_kb: None,
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                    failure: None,
                }],
            }],
        });

        assert!(markdown.contains(
            "| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |"
        ));
        assert!(markdown.contains(
            "| Google Pixel 8-14.0 | sample_fns::fibonacci | 4 | 1 | 1.000s | 4.000s | 200ms | 800ms | 20.0% | 1.00 MB | - |"
        ));
        assert!(!markdown.contains("### Device:"));
    }

    #[test]
    fn render_csv_summary_includes_resource_usage_columns() {
        let csv = render_csv_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(249_416),
                        peak_memory_growth_kb: Some(249_416),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                    failure: None,
                }],
            }],
        });

        assert!(
            csv.starts_with(
                "device,function,samples,mean_ns,median_ns,p95_ns,min_ns,max_ns,cpu_total_ms,cpu_median_ms,peak_memory_kb,peak_memory_growth_kb,process_peak_memory_kb\n"
            )
        );
        assert!(csv.contains(",482,241,249416,249416,1477787\n"));
    }

    #[test]
    fn render_csv_summary_quotes_records_and_neutralizes_formulas() {
        let csv = render_csv_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "unused".to_string(),
            iterations: 1,
            warmup: 0,
            devices: Vec::new(),
            device_summaries: vec![DeviceSummary {
                device: "\t=HYPERLINK(\"https://evil.invalid/a,b\")\r\nPixel".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "+SUM(1,2)".to_string(),
                    samples: 1,
                    mean_ns: Some(1),
                    median_ns: Some(1),
                    p95_ns: Some(1),
                    min_ns: Some(1),
                    max_ns: Some(1),
                    resource_usage: None,
                    failure: None,
                }],
            }],
        });

        assert_eq!(
            csv,
            concat!(
                "device,function,samples,mean_ns,median_ns,p95_ns,min_ns,max_ns,cpu_total_ms,cpu_median_ms,peak_memory_kb,peak_memory_growth_kb,process_peak_memory_kb\n",
                "\"'\t=HYPERLINK(\"\"https://evil.invalid/a,b\"\")\r\nPixel\",\"'+SUM(1,2)\",1,1,1,1,1,1,,,,,\n"
            )
        );
    }

    #[test]
    fn legacy_summary_csv_quotes_records_and_neutralizes_formulas() {
        let csv = render_summary_data_csv(&[SummaryData {
            source_file: "ignored".to_string(),
            function: Some("@cmd".to_string()),
            device: Some("Pixel, \"8\"".to_string()),
            os_version: Some("\n-1".to_string()),
            sample_count: 0,
            mean_ns: None,
            median_ns: None,
            min_ns: None,
            max_ns: None,
            p95_ns: None,
            iterations: None,
            warmup: None,
        }]);

        assert_eq!(
            csv,
            concat!(
                "function,device,os_version,sample_count,mean_ns,median_ns,min_ns,max_ns,p95_ns,iterations,warmup\n",
                "'@cmd,\"Pixel, \"\"8\"\"\",\"'\n-1\",0,,,,,,,\n"
            )
        );
    }

    #[test]
    fn failure_markdown_neutralizes_untrusted_report_fields() {
        let markdown = render_failure_markdown(&json!({
            "device": "Pixel|8\n# hacked",
            "function_name": "[run](x)",
            "kind": "timeout\n- injected",
            "message": "<script>alert(1)</script> ![x](y)",
            "elapsed_ms": 1_u64,
            "android_exit_info": { "reason": "@reason" }
        }));

        assert_eq!(
            markdown,
            concat!(
                "# Android Benchmark Failure\n\n",
                "- Device: Pixel&#124;8 &#35; hacked\n",
                "- Function: &#91;run&#93;&#40;x&#41;\n",
                "- Kind: timeout &#45; injected\n",
                "- Message: &lt;script&gt;alert&#40;1&#41;&lt;&#47;script&gt; &#33;&#91;x&#93;&#40;y&#41;\n",
                "- Elapsed: 1 ms\n",
                "- Exit reason: @reason\n"
            )
        );
    }

    #[test]
    fn summary_markdown_neutralizes_metadata_rows_and_failure_fields() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "<time>".to_string(),
            generated_at_unix: 1,
            target: MobileTarget::Android,
            function: "[root](x)".to_string(),
            iterations: 1,
            warmup: 0,
            devices: vec!["Pixel|8".to_string(), "Line\n#two".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "<img src=x>".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "![run](x)|next".to_string(),
                    samples: 0,
                    mean_ns: None,
                    median_ns: None,
                    p95_ns: None,
                    min_ns: None,
                    max_ns: None,
                    resource_usage: None,
                    failure: Some(BenchmarkFailureStats {
                        kind: "[timeout](x)".to_string(),
                        message: "unused".to_string(),
                        elapsed_ms: None,
                        exit_reason: Some("<script>".to_string()),
                    }),
                }],
            }],
        });

        assert!(markdown.contains("- Generated: &lt;time&gt;"));
        assert!(markdown.contains("- Function: &#91;root&#93;&#40;x&#41;"));
        assert!(markdown.contains("- Devices: Pixel&#124;8, Line &#35;two"));
        assert!(markdown.contains(concat!(
            "| &lt;img src=x&gt; | &#33;&#91;run&#93;&#40;x&#41;&#124;next | ",
            "failed (&#91;timeout&#93;&#40;x&#41;) |"
        )));
        assert!(markdown.contains("| - | &lt;script&gt; |"));
        assert!(!markdown.contains("<img"));
        assert!(!markdown.contains("![run]"));
    }

    #[test]
    fn comparison_markdown_neutralizes_paths_rows_and_labels() {
        let markdown = render_compare_markdown(&CompareReport {
            baseline: PathBuf::from("base\n# [link](x).json"),
            candidate: PathBuf::from("<script>.json"),
            rows: vec![CompareRow {
                device: "Pixel|8\n# row".to_string(),
                function: "![run](x)".to_string(),
                baseline_median_ns: None,
                candidate_median_ns: None,
                median_delta_pct: None,
                median_label: "[regressed](x)".to_string(),
                baseline_p95_ns: None,
                candidate_p95_ns: None,
                p95_delta_pct: None,
                p95_label: "<b>bad</b>".to_string(),
            }],
        });

        assert!(markdown.contains("- Baseline: base &#35; &#91;link&#93;&#40;x&#41;&#46;json"));
        assert!(markdown.contains("- Candidate: &lt;script&gt;&#46;json"));
        assert!(markdown.contains(concat!(
            "| Pixel&#124;8 &#35; row | &#33;&#91;run&#93;&#40;x&#41; | - | - | - | ",
            "&#91;regressed&#93;&#40;x&#41; | - | - | - | &lt;b&gt;bad&lt;&#47;b&gt; |"
        )));
        assert!(!markdown.contains("<script>"));
        assert!(!markdown.contains("![run]"));
    }

    #[test]
    fn merged_and_plot_markdown_neutralizes_targets_labels_and_destinations() {
        let merged = json!({
            "targets": {
                "android\n# [evil](x)": {
                    "summary": {
                        "generated_at": "2026-04-12T00:00:00Z",
                        "generated_at_unix": 1_u64,
                        "target": "android",
                        "function": "bench",
                        "iterations": 1_u64,
                        "warmup": 0_u64,
                        "devices": [],
                        "device_summaries": []
                    }
                }
            }
        });
        let merged_markdown =
            render_summary_markdown_from_output(&merged).expect("render merged summary");
        assert!(merged_markdown.starts_with("## android &#35; &#91;evil&#93;&#40;x&#41;\n\n"));

        let plot = plots::RenderedPlot {
            function_name: "unused".to_string(),
            function_label: "![evil](x)\n# heading".to_string(),
            target: "android".to_string(),
            output_path: PathBuf::from("unused"),
            relative_path: PathBuf::from("javascript:alert(1)\n).svg"),
        };
        let plot_markdown = append_plot_links_to_markdown(String::new(), &[&plot]);

        assert!(plot_markdown.contains(concat!(
            "### &#33;&#91;evil&#93;&#40;x&#41; &#35; heading\n",
            "![&#33;&#91;evil&#93;&#40;x&#41; &#35; heading]",
            "(javascript%3Aalert%281%29%0A%29.svg)\n"
        )));
        assert!(!plot_markdown.contains("javascript:"));
        assert!(!plot_markdown.contains("![evil]"));
    }

    #[test]
    fn render_summary_uses_legacy_peak_memory_as_growth_fallback() {
        let summary = SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(249_416),
                        peak_memory_growth_kb: None,
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                    failure: None,
                }],
            }],
        };

        let markdown = render_markdown_summary(&summary);
        let csv = render_csv_summary(&summary);

        assert!(markdown.contains("243.57 MB"));
        assert!(csv.contains(",482,241,249416,249416,1477787\n"));
    }

    #[test]
    fn test_render_markdown_uses_cpu_total_and_peak_memory_columns() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Google Pixel 8-14.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(654_321),
                        peak_memory_growth_kb: Some(654_321),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: Some(654_321),
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                    failure: None,
                }],
            }],
        });

        assert!(markdown.contains("CPU median / iter"));
        assert!(markdown.contains("CPU total"));
        assert!(markdown.contains("CPU / wall"));
        assert!(markdown.contains("Peak growth"));
        assert!(markdown.contains("Process peak"));
        assert!(!markdown.contains("Provider peak"));
        assert!(!markdown.contains("Absolute peak"));
        assert!(!markdown.contains("Peak memory"));
        assert!(markdown.contains("241ms"));
        assert!(markdown.contains("482ms"));
        assert!(markdown.contains("7.7%"));
        assert!(markdown.contains("638.99 MB"));
    }

    #[test]
    fn test_render_table_uses_cpu_total_and_peak_memory_columns() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Ios,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["iPhone 15-17.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "iPhone 15-17.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: Some(482),
                        cpu_median_ms: Some(241),
                        peak_memory_kb: Some(654_321),
                        peak_memory_growth_kb: Some(654_321),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                    failure: None,
                }],
            }],
        });

        assert!(markdown.contains("Device"));
        assert!(markdown.contains("Wall mean / iter"));
        assert!(markdown.contains("Wall total"));
        assert!(markdown.contains("CPU median / iter"));
        assert!(markdown.contains("CPU total"));
        assert!(markdown.contains("CPU / wall"));
        assert!(markdown.contains("Peak growth"));
        assert!(markdown.contains("Process peak"));
        assert!(!markdown.contains("Provider peak"));
        assert!(!markdown.contains("Absolute peak"));
        assert!(!markdown.contains("Peak memory"));
        assert!(markdown.contains("241ms"));
        assert!(markdown.contains("482ms"));
        assert!(markdown.contains("7.7%"));
        assert!(markdown.contains("638.99 MB"));
    }

    #[test]
    fn render_markdown_summary_notes_large_process_memory_baseline_gap() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            generated_at_unix: 1_744_416_000,
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["Motorola Moto G9 Play-11.0".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Motorola Moto G9 Play-11.0".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "sample_fns::fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(1_250_000_000),
                    median_ns: Some(1_200_000_000),
                    p95_ns: Some(1_300_000_000),
                    min_ns: Some(1_100_000_000),
                    max_ns: Some(1_350_000_000),
                    resource_usage: Some(BenchmarkResourceUsage {
                        cpu_total_ms: None,
                        cpu_median_ms: None,
                        peak_memory_kb: Some(171_556),
                        peak_memory_growth_kb: Some(171_556),
                        process_peak_memory_kb: Some(1_477_787),
                        total_pss_kb: Some(1_477_787),
                        private_dirty_kb: Some(1_462_460),
                        native_heap_kb: None,
                        java_heap_kb: None,
                    }),
                    failure: None,
                }],
            }],
        });

        assert!(markdown.contains("Peak growth"));
        assert!(markdown.contains("Process peak"));
        assert!(!markdown.contains("Provider peak"));
        assert!(!markdown.contains("Absolute peak"));
        assert!(markdown.contains(MEMORY_BASELINE_GAP_NOTE));
        assert!(!markdown.contains("Peak memory"));
    }

    #[test]
    fn build_summary_preserves_resource_usage_from_benchmark_results() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            iterations: 3,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".into()],
            browserstack: None,
            ios_xcuitest: None,
            ios_completion_timeout_secs: None,
            ios_deployment_target: None,
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
        };
        let run_summary = RunSummary {
            spec: spec.clone(),
            artifacts: None,
            local_report: json!({}),
            remote_run: None,
            summary: empty_summary(&spec),
            benchmark_results: Some(BTreeMap::from([(
                "Google Pixel 8-14.0".to_string(),
                vec![json!({
                    "function": "sample_fns::fibonacci",
                    "samples": [
                        { "duration_ns": 1000, "cpu_time_ms": 19, "peak_memory_kb": 48, "process_peak_memory_kb": 1048 },
                        { "duration_ns": 2000, "cpu_time_ms": 7, "peak_memory_kb": 96, "process_peak_memory_kb": 1096 },
                        { "duration_ns": 3000, "cpu_time_ms": 11, "peak_memory_kb": 64, "process_peak_memory_kb": 1064 }
                    ]
                })],
            )])),
            benchmark_failures: None,
            performance_metrics: None,
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let usage = summary.device_summaries[0].benchmarks[0]
            .resource_usage
            .as_ref()
            .expect("resource usage");

        assert_eq!(usage.cpu_total_ms, Some(37));
        assert_eq!(usage.cpu_median_ms, Some(11));
        assert_eq!(usage.peak_memory_kb, Some(96));
        assert_eq!(usage.peak_memory_growth_kb, Some(96));
        assert_eq!(usage.process_peak_memory_kb, Some(1_096));
    }

    #[test]
    fn build_summary_preserves_browserstack_failure_results() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "sample_fns::sleep".into(),
            iterations: 3,
            warmup: 1,
            devices: vec!["Vivo Y21-11.0".into()],
            browserstack: None,
            ios_xcuitest: None,
            ios_completion_timeout_secs: None,
            ios_deployment_target: None,
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
        };
        let run_summary = RunSummary {
            spec: spec.clone(),
            artifacts: None,
            local_report: json!({}),
            remote_run: None,
            summary: empty_summary(&spec),
            benchmark_results: None,
            benchmark_failures: Some(BTreeMap::from([(
                "Vivo Y21-11.0".to_string(),
                vec![json!({
                    "schema_version": 1,
                    "platform": "android",
                    "device": "Vivo Y21-11.0",
                    "function_name": "sample_fns::sleep",
                    "kind": "timeout",
                    "message": "Timed out waiting 30s for benchmark completion",
                    "elapsed_ms": 30_000_u64,
                    "memory": {
                        "total_pss_kb": 1024_u64
                    },
                    "android_exit_info": {
                        "reason": "low_memory",
                        "raw_reason": 3
                    }
                })],
            )])),
            performance_metrics: None,
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let benchmark = &summary.device_summaries[0].benchmarks[0];
        let failure = benchmark.failure.as_ref().expect("failure summary");
        let markdown = render_markdown_summary(&summary);

        assert_eq!(benchmark.function, "sample_fns::sleep");
        assert_eq!(benchmark.samples, 0);
        assert_eq!(failure.kind, "timeout");
        assert_eq!(failure.elapsed_ms, Some(30_000));
        assert_eq!(failure.exit_reason.as_deref(), Some("low_memory"));
        assert!(markdown.contains("failed (timeout)"));
        assert!(markdown.contains("low_memory"));
    }

    #[test]
    fn build_summary_prefers_measured_peak_memory_over_browserstack_perf_memory() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "sample_fns::fibonacci".into(),
            iterations: 2,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".into()],
            browserstack: None,
            ios_xcuitest: None,
            ios_completion_timeout_secs: None,
            ios_deployment_target: None,
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
        };
        let run_summary = RunSummary {
            spec: spec.clone(),
            artifacts: None,
            local_report: json!({}),
            remote_run: None,
            summary: empty_summary(&spec),
            benchmark_results: Some(BTreeMap::from([(
                "Google Pixel 8-14.0".to_string(),
                vec![json!({
                    "function": "sample_fns::fibonacci",
                    "samples": [
                        { "duration_ns": 1000, "cpu_time_ms": 10, "peak_memory_kb": 64, "process_peak_memory_kb": 1064 },
                        { "duration_ns": 2000, "cpu_time_ms": 12, "peak_memory_kb": 72, "process_peak_memory_kb": 1072 }
                    ]
                })],
            )])),
            benchmark_failures: None,
            performance_metrics: Some(BTreeMap::from([(
                "Google Pixel 8-14.0".to_string(),
                browserstack::PerformanceMetrics {
                    memory: Some(browserstack::AggregateMemoryMetrics {
                        peak_mb: 999.0,
                        average_mb: 900.0,
                        min_mb: 800.0,
                    }),
                    cpu: None,
                    sample_count: 1,
                    snapshots: vec![],
                },
            )])),
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let usage = summary.device_summaries[0].benchmarks[0]
            .resource_usage
            .as_ref()
            .expect("resource usage");

        assert_eq!(usage.peak_memory_kb, Some(72));
        assert_eq!(usage.peak_memory_growth_kb, Some(72));
        assert_eq!(usage.process_peak_memory_kb, Some(1_072));
    }

    #[test]
    fn format_cpu_total_duration_ms_uses_milliseconds_below_one_second() {
        assert_eq!(format_cpu_total_duration_ms(482), "482ms");
    }

    #[test]
    fn format_cpu_total_duration_ms_uses_total_seconds_at_or_above_one_second() {
        assert_eq!(format_cpu_total_duration_ms(1_000), "1.000s");
        assert_eq!(format_cpu_total_duration_ms(114_248), "114.248s");
        assert_eq!(format_cpu_total_duration_ms(515_822), "515.822s");
    }

    #[test]
    fn parse_pr_number_from_github_ref_extracts_pull_number() {
        assert_eq!(
            parse_pr_number_from_ref("refs/pull/123/merge"),
            Some("123".to_string())
        );
        assert_eq!(parse_pr_number_from_ref("refs/heads/main"), None);
    }

    #[test]
    fn contract_schema_files_compile() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let summary_schema_path = root.join("docs/schemas/summary-v1.schema.json");
        let ci_schema_path = root.join("docs/schemas/ci-contract-v1.schema.json");
        let trace_schema_path = root.join("docs/schemas/trace-events-v1.schema.json");

        let summary_schema: Value = serde_json::from_str(
            &fs::read_to_string(&summary_schema_path).expect("read summary schema"),
        )
        .expect("parse summary schema");
        let ci_schema: Value =
            serde_json::from_str(&fs::read_to_string(&ci_schema_path).expect("read ci schema"))
                .expect("parse ci schema");
        let trace_schema: Value = serde_json::from_str(
            &fs::read_to_string(&trace_schema_path).expect("read trace schema"),
        )
        .expect("parse trace schema");

        JSONSchema::options()
            .compile(&summary_schema)
            .expect("compile summary schema");
        JSONSchema::options()
            .compile(&ci_schema)
            .expect("compile ci schema");
        JSONSchema::options()
            .compile(&trace_schema)
            .expect("compile trace schema");
    }

    #[test]
    fn run_summary_validates_against_summary_schema() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let summary_schema_path = root.join("docs/schemas/summary-v1.schema.json");
        let summary_schema: Value = serde_json::from_str(
            &fs::read_to_string(&summary_schema_path).expect("read summary schema"),
        )
        .expect("parse summary schema");
        let validator = JSONSchema::options()
            .compile(&summary_schema)
            .expect("compile summary schema");

        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "noop_benchmark".into(),
            iterations: 3,
            warmup: 1,
            devices: vec![],
            ios_completion_timeout_secs: None,
            ios_deployment_target: None,
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
            browserstack: None,
            ios_xcuitest: None,
        };
        let local_report = run_local_smoke(&spec).expect("local harness");
        let mut run_summary = RunSummary {
            spec,
            artifacts: None,
            local_report,
            remote_run: None,
            summary: empty_summary(&RunSpec {
                target: MobileTarget::Android,
                function: "noop_benchmark".into(),
                iterations: 3,
                warmup: 1,
                devices: vec![],
                ios_completion_timeout_secs: None,
                ios_deployment_target: None,
                ios_runner: None,
                android_benchmark_timeout_secs: None,
                android_heartbeat_interval_secs: None,
                browserstack: None,
                ios_xcuitest: None,
            }),
            benchmark_results: None,
            benchmark_failures: None,
            performance_metrics: None,
        };
        run_summary.summary = build_summary(&run_summary).expect("build summary");
        let value = serde_json::to_value(&run_summary).expect("serialize run summary");

        if let Err(errors) = validator.validate(&value) {
            let messages: Vec<String> = errors.map(|e| e.to_string()).collect();
            panic!("summary schema validation failed: {}", messages.join(" | "));
        }
    }

    #[test]
    fn ci_payload_validates_against_ci_schema() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let ci_schema_path = root.join("docs/schemas/ci-contract-v1.schema.json");
        let ci_schema: Value =
            serde_json::from_str(&fs::read_to_string(&ci_schema_path).expect("read ci schema"))
                .expect("parse ci schema");
        let validator = JSONSchema::options()
            .compile(&ci_schema)
            .expect("compile ci schema");

        let payload = json!({
            "ci": {
                "metadata": {
                    "requested_by": "codex",
                    "pr_number": "123",
                    "request_command": "cargo mobench ci run --target android --function noop_benchmark",
                    "mobench_ref": "refs/heads/codex/ci-devex",
                    "mobench_version": env!("CARGO_PKG_VERSION")
                },
                "outputs": {
                    "summary_json": "target/mobench/ci/summary.json",
                    "summary_md": "target/mobench/ci/summary.md",
                    "results_csv": "target/mobench/ci/results.csv"
                }
            }
        });

        if let Err(errors) = validator.validate(&payload) {
            let messages: Vec<String> = errors.map(|e| e.to_string()).collect();
            panic!("ci schema validation failed: {}", messages.join(" | "));
        }
    }

    #[test]
    fn example_summary_fixtures_validate_against_summary_schema() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let summary_schema_path = root.join("docs/schemas/summary-v1.schema.json");
        let summary_schema: Value = serde_json::from_str(
            &fs::read_to_string(&summary_schema_path).expect("read summary schema"),
        )
        .expect("parse summary schema");
        let validator = JSONSchema::options()
            .compile(&summary_schema)
            .expect("compile summary schema");

        for fixture in [
            "examples/fixtures/basic/summary.json",
            "examples/fixtures/ffi/summary.json",
            "crates/mobench/tests/fixtures/ci-artifact-root/android/summary.json",
        ] {
            let fixture_path = root.join(fixture);
            let value: Value = serde_json::from_str(
                &fs::read_to_string(&fixture_path).expect("read summary fixture"),
            )
            .expect("parse summary fixture");

            if let Err(errors) = validator.validate(&value) {
                let messages: Vec<String> = errors.map(|e| e.to_string()).collect();
                panic!(
                    "{} failed summary schema validation: {}",
                    fixture_path.display(),
                    messages.join(" | ")
                );
            }
        }
    }

    #[test]
    fn example_trace_events_fixture_validates_against_trace_schema() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let trace_schema_path = root.join("docs/schemas/trace-events-v1.schema.json");
        let trace_schema: Value = serde_json::from_str(
            &fs::read_to_string(&trace_schema_path).expect("read trace schema"),
        )
        .expect("parse trace schema");
        let validator = JSONSchema::options()
            .compile(&trace_schema)
            .expect("compile trace schema");

        let fixture_path = root.join("examples/fixtures/profile/trace-events.json");
        let value: Value =
            serde_json::from_str(&fs::read_to_string(&fixture_path).expect("read trace fixture"))
                .expect("parse trace fixture");

        if let Err(errors) = validator.validate(&value) {
            let messages: Vec<String> = errors.map(|e| e.to_string()).collect();
            panic!(
                "{} failed trace schema validation: {}",
                fixture_path.display(),
                messages.join(" | ")
            );
        }
    }

    #[test]
    fn basic_example_fixture_renders_stable_markdown_and_csv() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let fixture_path = root.join("examples/fixtures/basic/summary.json");
        let value: Value =
            serde_json::from_str(&fs::read_to_string(&fixture_path).expect("read fixture"))
                .expect("parse fixture");
        let summary = summary_report_from_value(&value).expect("parse summary report");

        let markdown = render_markdown_summary(&summary);
        assert_eq!(
            markdown,
            "\
### Benchmark Summary

- Generated: 2026-03-26T00:00:00Z
- Target: Android
- Function: multiple
- Iterations/Warmup: 5 / 1
- Devices: Google Pixel 8-14.0, Samsung Galaxy S23-14.0

| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Google Pixel 8-14.0 | basic_benchmark::bench_fibonacci | 5 | 1 | 100.000ms | 500.000ms | - | - | - | - | - |
| Google Pixel 8-14.0 | basic_benchmark::bench_checksum | 5 | 1 | 145.000ms | 725.000ms | - | - | - | - | - |
| Samsung Galaxy S23-14.0 | basic_benchmark::bench_fibonacci | 5 | 1 | 94.000ms | 470.000ms | - | - | - | - | - |
| Samsung Galaxy S23-14.0 | basic_benchmark::bench_checksum | 5 | 1 | 136.000ms | 680.000ms | - | - | - | - | - |

"
        );

        let csv = render_csv_summary(&summary);
        assert_eq!(
            csv,
            "\
device,function,samples,mean_ns,median_ns,p95_ns,min_ns,max_ns,cpu_total_ms,cpu_median_ms,peak_memory_kb,peak_memory_growth_kb,process_peak_memory_kb
Google Pixel 8-14.0,basic_benchmark::bench_fibonacci,5,100000000,100000000,105000000,95000000,105000000,,,,,
Google Pixel 8-14.0,basic_benchmark::bench_checksum,5,145000000,145000000,151000000,140000000,151000000,,,,,
Samsung Galaxy S23-14.0,basic_benchmark::bench_fibonacci,5,94000000,94000000,98000000,90000000,98000000,,,,,
Samsung Galaxy S23-14.0,basic_benchmark::bench_checksum,5,136000000,136000000,140000000,132000000,140000000,,,,,
"
        );
    }

    #[test]
    fn ci_function_slug_distinguishes_ambiguous_paths() {
        assert_ne!(ci_function_slug("a::b_c"), ci_function_slug("a_b::c"));
    }

    #[test]
    fn baseline_lookup_matches_device_row() {
        let baseline_report = summarize::SummarizeReport {
            platforms: vec![
                summarize::PlatformReport {
                    platform: "android".to_string(),
                    device: summarize::DeviceInfo {
                        name: "Google Pixel 6".to_string(),
                        os: "Android".to_string(),
                        os_version: "14".to_string(),
                        chipset: None,
                        ram_gb: None,
                    },
                    benchmarks: vec![summarize::BenchmarkResult {
                        name: "bench_alpha".to_string(),
                        label: "alpha".to_string(),
                        timing: summarize::TimingStats {
                            avg_ms: 100.0,
                            median_ms: 100.0,
                            best_ms: 100.0,
                            worst_ms: 100.0,
                            p95_ms: 100.0,
                            std_dev_ms: None,
                        },
                        resource_usage: None,
                        failure: None,
                    }],
                    iterations: 5,
                    warmup: 1,
                },
                summarize::PlatformReport {
                    platform: "android".to_string(),
                    device: summarize::DeviceInfo {
                        name: "Samsung Galaxy S24".to_string(),
                        os: "Android".to_string(),
                        os_version: "14".to_string(),
                        chipset: None,
                        ram_gb: None,
                    },
                    benchmarks: vec![summarize::BenchmarkResult {
                        name: "bench_alpha".to_string(),
                        label: "alpha".to_string(),
                        timing: summarize::TimingStats {
                            avg_ms: 200.0,
                            median_ms: 200.0,
                            best_ms: 200.0,
                            worst_ms: 200.0,
                            p95_ms: 200.0,
                            std_dev_ms: None,
                        },
                        resource_usage: None,
                        failure: None,
                    }],
                    iterations: 5,
                    warmup: 1,
                },
            ],
        };

        let baseline = find_baseline_benchmark(
            &baseline_report,
            "android",
            "Samsung Galaxy S24",
            "14",
            "bench_alpha",
        )
        .expect("matching baseline benchmark");

        assert_eq!(baseline.timing.avg_ms, 200.0);
    }
}

#[cfg(test)]
mod result_extraction_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_all_benchmark_results() {
        let results: HashMap<String, Vec<serde_json::Value>> = [
            (
                "Pixel 7".to_string(),
                vec![json!({
                    "function": "my_crate::bench_fn",
                    "mean_ns": 12345678,
                    "samples": [{"duration_ns": 12345678}]
                })],
            ),
            (
                "iPhone 14".to_string(),
                vec![json!({
                    "function": "my_crate::bench_fn",
                    "mean_ns": 11111111,
                    "samples": [{"duration_ns": 11111111}]
                })],
            ),
        ]
        .into_iter()
        .collect();

        let extracted = extract_benchmark_summary(&results);
        assert_eq!(extracted.len(), 2);
        assert!(extracted.iter().any(|r| r.device == "Pixel 7"));
        assert!(extracted.iter().any(|r| r.device == "iPhone 14"));
    }

    #[test]
    fn test_extract_with_multiple_samples() {
        let results: HashMap<String, Vec<serde_json::Value>> = [(
            "Device".to_string(),
            vec![json!({
                "function": "test_fn",
                "mean_ns": 999,
                "samples": [
                    {"duration_ns": 80},
                    {"duration_ns": 100},
                    {"duration_ns": 120}
                ]
            })],
        )]
        .into_iter()
        .collect();

        let extracted = extract_benchmark_summary(&results);
        assert_eq!(extracted.len(), 1);
        let result = &extracted[0];
        assert_eq!(result.sample_count, 3);
        assert_eq!(result.mean_ns, 100);
        assert_eq!(result.min_ns, Some(80));
        assert_eq!(result.max_ns, Some(120));
        assert_eq!(result.std_dev_ns, Some(20));
    }
}

#[cfg(test)]
mod ci_merge_tests {
    use super::*;
    use serde_json::json;

    fn sample_run_summary(
        target: MobileTarget,
        function: &str,
        device: &str,
        mean_ns: u64,
    ) -> Value {
        json!({
            "summary": {
                "generated_at": "2026-02-16T00:00:00Z",
                "generated_at_unix": 1708041600,
                "target": target.as_str(),
                "function": function,
                "iterations": 3,
                "warmup": 1,
                "devices": [device],
                "device_summaries": [{
                    "device": device,
                    "benchmarks": [{
                        "function": function,
                        "samples": 3,
                        "mean_ns": mean_ns,
                        "median_ns": mean_ns,
                        "p95_ns": mean_ns,
                        "min_ns": mean_ns,
                        "max_ns": mean_ns
                    }]
                }]
            }
        })
    }

    #[test]
    fn merge_ci_target_runs_preserves_all_functions() {
        let runs = BTreeMap::from([
            (
                "bench_a".to_string(),
                sample_run_summary(MobileTarget::Ios, "bench_a", "iPhone 14-16.0", 100),
            ),
            (
                "bench_b".to_string(),
                sample_run_summary(MobileTarget::Ios, "bench_b", "iPhone 14-16.0", 200),
            ),
        ]);

        let merged = merge_ci_target_runs(MobileTarget::Ios, &runs).unwrap();
        let functions = merged
            .get("functions")
            .and_then(|v| v.as_object())
            .expect("functions map");
        assert_eq!(functions.len(), 2);

        let benchmarks = merged["summary"]["device_summaries"][0]["benchmarks"]
            .as_array()
            .expect("benchmarks");
        assert_eq!(benchmarks.len(), 2);
        assert_eq!(benchmarks[0]["function"], "bench_a");
        assert_eq!(benchmarks[1]["function"], "bench_b");
    }

    #[test]
    fn root_summary_from_merged_targets_returns_summary_for_single_target() {
        let merged_target = merge_ci_target_runs(
            MobileTarget::Ios,
            &BTreeMap::from([(
                "bench_a".to_string(),
                sample_run_summary(MobileTarget::Ios, "bench_a", "iPhone 14-16.0", 100),
            )]),
        )
        .unwrap();
        let targets = BTreeMap::from([("ios".to_string(), merged_target)]);

        let root_summary = root_summary_from_merged_targets(&targets).expect("single target");
        assert_eq!(root_summary["target"], "ios");
        assert_eq!(
            root_summary["device_summaries"][0]["benchmarks"][0]["function"],
            "bench_a"
        );
    }

    #[test]
    fn merge_ci_target_runs_preserves_resource_usage() {
        let runs = BTreeMap::from([
            (
                "bench_a".to_string(),
                json!({
                    "summary": {
                        "generated_at": "2026-02-16T00:00:00Z",
                        "generated_at_unix": 1708041600,
                        "target": "android",
                        "function": "bench_a",
                        "iterations": 3,
                        "warmup": 1,
                        "devices": ["Pixel 8-14.0"],
                        "device_summaries": [{
                            "device": "Pixel 8-14.0",
                            "benchmarks": [{
                                "function": "bench_a",
                                "samples": 3,
                                "mean_ns": 100,
                                "median_ns": 100,
                                "p95_ns": 100,
                                "min_ns": 100,
                                "max_ns": 100,
                                "resource_usage": {
                                    "cpu_total_ms": 482,
                                    "peak_memory_kb": 654321,
                                    "total_pss_kb": 654321
                                }
                            }]
                        }]
                    }
                }),
            ),
            (
                "bench_b".to_string(),
                sample_run_summary(MobileTarget::Android, "bench_b", "Pixel 8-14.0", 200),
            ),
        ]);

        let merged = merge_ci_target_runs(MobileTarget::Android, &runs).expect("merge targets");
        let benchmarks = merged["summary"]["device_summaries"][0]["benchmarks"]
            .as_array()
            .expect("benchmarks");
        let bench_a = benchmarks
            .iter()
            .find(|benchmark| benchmark["function"] == "bench_a")
            .expect("bench_a");

        assert_eq!(bench_a["resource_usage"]["cpu_total_ms"], 482);
        assert_eq!(bench_a["resource_usage"]["peak_memory_kb"], 654321);
    }

    #[test]
    fn render_summary_markdown_from_output_renders_all_functions_from_merged_targets() {
        let ios = merge_ci_target_runs(
            MobileTarget::Ios,
            &BTreeMap::from([
                (
                    "bench_a".to_string(),
                    sample_run_summary(MobileTarget::Ios, "bench_a", "iPhone 14-16.0", 100),
                ),
                (
                    "bench_b".to_string(),
                    sample_run_summary(MobileTarget::Ios, "bench_b", "iPhone 14-16.0", 200),
                ),
            ]),
        )
        .unwrap();
        let android = merge_ci_target_runs(
            MobileTarget::Android,
            &BTreeMap::from([(
                "bench_c".to_string(),
                sample_run_summary(MobileTarget::Android, "bench_c", "Pixel 7-14.0", 300),
            )]),
        )
        .unwrap();

        let markdown = render_summary_markdown_from_output(&json!({
            "targets": {
                "ios": ios,
                "android": android
            }
        }))
        .unwrap();

        assert!(markdown.contains("## ios"));
        assert!(markdown.contains("## android"));
        assert!(markdown.contains("bench_a"));
        assert!(markdown.contains("bench_b"));
        assert!(markdown.contains("bench_c"));
    }

    #[test]
    fn render_markdown_summary_uses_h3_heading_and_ios_label() {
        let markdown = render_markdown_summary(&SummaryReport {
            generated_at: "2026-03-27T00:45:55.028899Z".to_string(),
            generated_at_unix: 1_774_569_955,
            target: MobileTarget::Ios,
            function: "ffi_benchmark::bench_fibonacci".to_string(),
            iterations: 5,
            warmup: 1,
            devices: vec!["iPhone 13-15".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "iPhone 13".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "ffi_benchmark::bench_fibonacci".to_string(),
                    samples: 5,
                    mean_ns: Some(17_000),
                    median_ns: Some(17_000),
                    p95_ns: Some(18_000),
                    min_ns: Some(16_000),
                    max_ns: Some(19_000),
                    resource_usage: None,
                    failure: None,
                }],
            }],
        });

        assert!(markdown.starts_with("### Benchmark Summary\n"));
        assert!(markdown.contains("- Target: iOS"));
        assert!(markdown.contains("| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |"));
        assert!(markdown.contains("| iPhone 13 | ffi_benchmark::bench_fibonacci | 5 | 1 | 0.017ms | 0.085ms | - | - | - | - | - |"));
        assert!(!markdown.contains("### Device:"));
    }

    #[cfg(unix)]
    #[test]
    fn render_summary_markdown_from_output_with_plots_embeds_image_links() {
        let output = json!({
            "summary": {
                "generated_at": "2026-03-25T00:00:00Z",
                "generated_at_unix": 1_742_862_400_u64,
                "target": "android",
                "function": "bench_alpha",
                "iterations": 3,
                "warmup": 1,
                "devices": ["Google Pixel 8-14.0", "iPhone 15-17.4"],
                "device_summaries": [
                    {
                        "device": "Google Pixel 8-14.0",
                        "benchmarks": [{
                            "function": "bench_alpha",
                            "samples": 3,
                            "mean_ns": 97_u64,
                            "median_ns": 98_u64,
                            "p95_ns": 100_u64,
                            "min_ns": 95_u64,
                            "max_ns": 100_u64
                        }]
                    },
                    {
                        "device": "iPhone 15-17.4",
                        "benchmarks": [{
                            "function": "bench_alpha",
                            "samples": 3,
                            "mean_ns": 82_u64,
                            "median_ns": 82_u64,
                            "p95_ns": 84_u64,
                            "min_ns": 80_u64,
                            "max_ns": 84_u64
                        }]
                    }
                ]
            },
            "benchmark_results": {
                "Google Pixel 8-14.0": [{
                    "function": "bench_alpha",
                    "samples": [95_u64, 98_u64, 100_u64]
                }],
                "iPhone 15-17.4": [{
                    "function": "bench_alpha",
                    "samples": [80_u64, 82_u64, 84_u64]
                }]
            }
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let fake_python = crate::tests::write_fake_plot_python(dir.path());

        let markdown = render_summary_markdown_from_output_with_plots_using_python(
            &output,
            dir.path(),
            plots::PlotMode::Require,
            Some(&fake_python),
        )
        .expect("render markdown with plots");

        assert!(markdown.contains("### Device Comparison Plots"));
        assert!(markdown.contains("![alpha](plots/alpha.svg)"));
        assert!(dir.path().join("plots/alpha.svg").exists());
    }

    #[cfg(unix)]
    #[test]
    fn render_summary_markdown_from_output_with_plots_deduplicates_across_targets() {
        let merged = json!({
            "targets": {
                "android": {
                    "summary": {
                        "generated_at": "2026-03-25T00:00:00Z",
                        "generated_at_unix": 1_742_862_400_u64,
                        "target": "android",
                        "function": "bench_alpha",
                        "iterations": 3,
                        "warmup": 1,
                        "devices": ["Google Pixel 8-14.0"],
                        "device_summaries": [{
                            "device": "Google Pixel 8-14.0",
                            "benchmarks": [{
                                "function": "bench_alpha",
                                "samples": 3,
                                "mean_ns": 97_u64,
                                "median_ns": 98_u64,
                                "p95_ns": 100_u64,
                                "min_ns": 95_u64,
                                "max_ns": 100_u64
                            }]
                        }]
                    },
                    "functions": {
                        "bench_alpha": {
                            "summary": {
                                "generated_at": "2026-03-25T00:00:00Z",
                                "generated_at_unix": 1_742_862_400_u64,
                                "target": "android",
                                "function": "bench_alpha",
                                "iterations": 3,
                                "warmup": 1,
                                "devices": ["Google Pixel 8-14.0"],
                                "device_summaries": [{
                                    "device": "Google Pixel 8-14.0",
                                    "benchmarks": [{
                                        "function": "bench_alpha",
                                        "samples": 3,
                                        "mean_ns": 97_u64,
                                        "median_ns": 98_u64,
                                        "p95_ns": 100_u64,
                                        "min_ns": 95_u64,
                                        "max_ns": 100_u64
                                    }]
                                }]
                            },
                            "benchmark_results": {
                                "Google Pixel 8-14.0": [{
                                    "function": "bench_alpha",
                                    "samples": [95_u64, 98_u64, 100_u64]
                                }]
                            }
                        }
                    }
                },
                "ios": {
                    "summary": {
                        "generated_at": "2026-03-25T00:00:00Z",
                        "generated_at_unix": 1_742_862_400_u64,
                        "target": "ios",
                        "function": "bench_alpha",
                        "iterations": 3,
                        "warmup": 1,
                        "devices": ["iPhone 15-17.4"],
                        "device_summaries": [{
                            "device": "iPhone 15-17.4",
                            "benchmarks": [{
                                "function": "bench_alpha",
                                "samples": 3,
                                "mean_ns": 82_u64,
                                "median_ns": 82_u64,
                                "p95_ns": 84_u64,
                                "min_ns": 80_u64,
                                "max_ns": 84_u64
                            }]
                        }]
                    },
                    "functions": {
                        "bench_alpha": {
                            "summary": {
                                "generated_at": "2026-03-25T00:00:00Z",
                                "generated_at_unix": 1_742_862_400_u64,
                                "target": "ios",
                                "function": "bench_alpha",
                                "iterations": 3,
                                "warmup": 1,
                                "devices": ["iPhone 15-17.4"],
                                "device_summaries": [{
                                    "device": "iPhone 15-17.4",
                                    "benchmarks": [{
                                        "function": "bench_alpha",
                                        "samples": 3,
                                        "mean_ns": 82_u64,
                                        "median_ns": 82_u64,
                                        "p95_ns": 84_u64,
                                        "min_ns": 80_u64,
                                        "max_ns": 84_u64
                                    }]
                                }]
                            },
                            "benchmark_results": {
                                "iPhone 15-17.4": [{
                                    "function": "bench_alpha",
                                    "samples": [80_u64, 82_u64, 84_u64]
                                }]
                            }
                        }
                    }
                }
            }
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let fake_python = crate::tests::write_fake_plot_python(dir.path());

        let markdown = render_summary_markdown_from_output_with_plots_using_python(
            &merged,
            dir.path(),
            plots::PlotMode::Require,
            Some(&fake_python),
        )
        .expect("render merged markdown with plots");

        assert!(markdown.contains("## android"));
        assert!(markdown.contains("## ios"));
        assert!(markdown.contains("![alpha](plots/alpha.svg)"));
        assert!(markdown.contains("![alpha](plots/alpha-ios.svg)"));
        assert!(dir.path().join("plots/alpha.svg").exists());
        assert!(dir.path().join("plots/alpha-ios.svg").exists());
    }

    #[test]
    fn build_summary_preserves_resource_usage_from_benchmark_results() {
        let spec = RunSpec {
            target: MobileTarget::Android,
            function: "bench_nullifier_proving_only".into(),
            iterations: 3,
            warmup: 1,
            devices: vec!["Google Pixel 8-14.0".into()],
            browserstack: None,
            ios_xcuitest: None,
            ios_completion_timeout_secs: None,
            ios_deployment_target: None,
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
        };
        let local_report = json!({});
        let run_summary = RunSummary {
            spec: spec.clone(),
            artifacts: None,
            local_report,
            remote_run: None,
            summary: empty_summary(&spec),
            benchmark_results: Some(BTreeMap::from([(
                "Google Pixel 8-14.0".to_string(),
                vec![json!({
                    "function": "bench_nullifier_proving_only",
                    "mean_ns": 125000000_u64,
                    "samples": [
                        { "duration_ns": 120000000_u64 },
                        { "duration_ns": 130000000_u64 }
                    ],
                    "resources": {
                        "elapsed_cpu_ms": 482,
                        "total_pss_kb": 654321,
                        "private_dirty_kb": 321000,
                        "native_heap_kb": 120000,
                        "java_heap_kb": 45000
                    }
                })],
            )])),
            benchmark_failures: None,
            performance_metrics: None,
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let value = serde_json::to_value(summary).expect("serialize summary");
        let resource_usage = &value["device_summaries"][0]["benchmarks"][0]["resource_usage"];

        assert_eq!(resource_usage["cpu_total_ms"], 482);
        assert_eq!(resource_usage["peak_memory_kb"], Value::Null);
        assert_eq!(resource_usage["peak_memory_growth_kb"], Value::Null);
        assert_eq!(resource_usage["process_peak_memory_kb"], Value::Null);
        assert_eq!(resource_usage["total_pss_kb"], 654321);
        assert_eq!(resource_usage["private_dirty_kb"], 321000);
        assert_eq!(resource_usage["native_heap_kb"], 120000);
        assert_eq!(resource_usage["java_heap_kb"], 45000);
    }

    #[test]
    fn build_summary_ignores_browserstack_peak_memory_for_ci_summary() {
        let spec = RunSpec {
            target: MobileTarget::Ios,
            function: "bench_nullifier_proving_only".into(),
            iterations: 3,
            warmup: 1,
            devices: vec!["iPhone 15-17.0".into()],
            browserstack: None,
            ios_xcuitest: None,
            ios_completion_timeout_secs: None,
            ios_deployment_target: None,
            ios_runner: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
        };
        let run_summary = RunSummary {
            spec: spec.clone(),
            artifacts: None,
            local_report: json!({}),
            remote_run: None,
            summary: empty_summary(&spec),
            benchmark_results: Some(BTreeMap::from([(
                "iPhone 15-17.0".to_string(),
                vec![json!({
                    "function": "bench_nullifier_proving_only",
                    "mean_ns": 125000000_u64,
                    "samples": [
                        { "duration_ns": 120000000_u64 },
                        { "duration_ns": 130000000_u64 }
                    ],
                    "resources": {
                        "platform": "ios"
                    }
                })],
            )])),
            benchmark_failures: None,
            performance_metrics: Some(BTreeMap::from([(
                "iPhone 15-17.0".to_string(),
                browserstack::PerformanceMetrics {
                    sample_count: 1,
                    memory: Some(browserstack::AggregateMemoryMetrics {
                        peak_mb: 243.57,
                        average_mb: 169.45,
                        min_mb: 169.45,
                    }),
                    cpu: Some(browserstack::AggregateCpuMetrics {
                        peak_percent: 12.52,
                        average_percent: 5.06,
                        min_percent: 5.06,
                    }),
                    snapshots: Vec::new(),
                },
            )])),
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let value = serde_json::to_value(summary).expect("serialize summary");
        let benchmark = &value["device_summaries"][0]["benchmarks"][0];

        assert_eq!(benchmark["resource_usage"], Value::Null);
    }
}

#[cfg(test)]
mod init_sdk_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_sdk_creates_mobench_toml() {
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("my-bench");

        // Run init-sdk
        cmd_init_sdk(
            SdkTarget::Android,
            "my-bench".to_string(),
            output_dir.clone(),
            false,
        )
        .unwrap();

        // Check mobench.toml was created
        let config_path = output_dir.join("mobench.toml");
        assert!(
            config_path.exists(),
            "mobench.toml should be created by init-sdk"
        );

        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            contents.contains("my-bench"),
            "Config should contain project name"
        );
        assert!(
            contents.contains("[project]"),
            "Config should have [project] section"
        );
        assert!(
            contents.contains("[benchmarks]"),
            "Config should have [benchmarks] section"
        );
    }

    #[test]
    fn test_init_sdk_mobench_toml_has_correct_library_name() {
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("my-project");

        cmd_init_sdk(
            SdkTarget::Android,
            "my-project".to_string(),
            output_dir.clone(),
            false,
        )
        .unwrap();

        let config_path = output_dir.join("mobench.toml");
        let contents = std::fs::read_to_string(&config_path).unwrap();

        // Library name should have hyphens replaced with underscores
        assert!(
            contents.contains("library_name = \"my_project\""),
            "Config should have library_name with underscores"
        );
    }
}

#[cfg(test)]
mod resource_usage_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_resource_usage_from_entry_fields() {
        let entry = json!({
            "resources": {
                "elapsed_cpu_ms": 120,
                "total_pss_kb": 4096,
                "private_dirty_kb": 2048,
                "native_heap_kb": 1024,
                "java_heap_kb": 512
            }
        });

        let usage = extract_benchmark_resource_usage(&entry, None).unwrap();
        assert_eq!(usage.cpu_total_ms, Some(120));
        assert_eq!(usage.total_pss_kb, Some(4096));
        assert_eq!(usage.private_dirty_kb, Some(2048));
        assert_eq!(usage.native_heap_kb, Some(1024));
        assert_eq!(usage.java_heap_kb, Some(512));
        assert_eq!(usage.peak_memory_kb, None);
        assert_eq!(usage.peak_memory_growth_kb, None);
        assert_eq!(usage.process_peak_memory_kb, None);
    }

    #[test]
    fn test_extract_resource_usage_ignores_provider_peak() {
        let entry = json!({
            "resources": {
                "total_pss_kb": 4096
            }
        });
        let perf = browserstack::PerformanceMetrics {
            sample_count: 5,
            memory: Some(browserstack::AggregateMemoryMetrics {
                peak_mb: 10.0,
                average_mb: 8.0,
                min_mb: 6.0,
            }),
            cpu: None,
            snapshots: vec![],
        };

        let usage = extract_benchmark_resource_usage(&entry, Some(&perf)).unwrap();
        assert_eq!(usage.peak_memory_kb, None);
        assert_eq!(usage.peak_memory_growth_kb, None);
        assert_eq!(usage.process_peak_memory_kb, None);
        assert_eq!(usage.total_pss_kb, Some(4096));
    }

    #[test]
    fn test_extract_resource_usage_preserves_moto_growth_and_process_peak() {
        let entry = json!({
            "resources": {
                "peak_memory_kb": 171556,
                "process_peak_memory_kb": 1477787,
                "total_pss_kb": 1477787,
                "private_dirty_kb": 1462460,
                "native_heap_kb": 532000,
                "java_heap_kb": 212000
            }
        });
        let perf = browserstack::PerformanceMetrics {
            sample_count: 5,
            memory: Some(browserstack::AggregateMemoryMetrics {
                peak_mb: 1640.65,
                average_mb: 1500.0,
                min_mb: 1400.0,
            }),
            cpu: None,
            snapshots: vec![],
        };

        let usage = extract_benchmark_resource_usage(&entry, Some(&perf)).unwrap();

        assert_eq!(usage.peak_memory_growth_kb, Some(171_556));
        assert_eq!(usage.peak_memory_kb, Some(171_556));
        assert_eq!(usage.process_peak_memory_kb, Some(1_477_787));
        assert_eq!(usage.total_pss_kb, Some(1_477_787));
        assert_eq!(usage.private_dirty_kb, Some(1_462_460));
        assert_eq!(usage.native_heap_kb, Some(532_000));
        assert_eq!(usage.java_heap_kb, Some(212_000));
    }

    #[test]
    fn test_extract_resource_usage_empty_returns_none() {
        let entry = json!({});
        let usage = extract_benchmark_resource_usage(&entry, None);
        assert!(usage.is_none());
    }

    #[test]
    fn test_resource_usage_json_round_trip() {
        let usage = BenchmarkResourceUsage {
            cpu_total_ms: Some(250),
            cpu_median_ms: Some(125),
            peak_memory_kb: Some(8192),
            peak_memory_growth_kb: Some(8192),
            process_peak_memory_kb: Some(12288),
            total_pss_kb: Some(4096),
            private_dirty_kb: Some(2048),
            native_heap_kb: Some(1024),
            java_heap_kb: None,
        };

        let json_str = serde_json::to_string(&usage).unwrap();
        let deserialized: BenchmarkResourceUsage = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.cpu_total_ms, Some(250));
        assert_eq!(deserialized.cpu_median_ms, Some(125));
        assert_eq!(deserialized.peak_memory_kb, Some(8192));
        assert_eq!(deserialized.peak_memory_growth_kb, Some(8192));
        assert_eq!(deserialized.process_peak_memory_kb, Some(12288));
        assert_eq!(deserialized.total_pss_kb, Some(4096));
        assert_eq!(deserialized.private_dirty_kb, Some(2048));
        assert_eq!(deserialized.native_heap_kb, Some(1024));
        assert_eq!(deserialized.java_heap_kb, None);

        // java_heap_kb should be absent in JSON due to skip_serializing_if
        assert!(!json_str.contains("java_heap_kb"));
        assert!(json_str.contains("peak_memory_kb"));
        assert!(json_str.contains("peak_memory_growth_kb"));
        assert!(json_str.contains("process_peak_memory_kb"));
        assert!(!json_str.contains("absolute_peak_memory_kb"));
    }
}
