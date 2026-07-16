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
//! - **Building** - Compiles Rust code for Android/iOS and packages mobile apps
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
//! | `run` | Execute benchmarks locally or on devices |
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
    RegressionFinding, SummaryReport as CanonicalSummaryReport, csv_field, detect_regressions,
    render_compare_markdown,
};
use mobench_runtime::Distribution;
#[cfg(test)]
use mobench_runtime::MAX_BENCHMARK_COUNT;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

use browserstack::{
    BrowserStackArtifacts, BrowserStackAuth, BrowserStackClient, BrowserStackPlatform,
    BrowserStackProviderAdapter, BrowserStackRunHandle, BrowserStackRunRequest,
    DEFAULT_BROWSERSTACK_FETCH_TIMEOUT_SECS, completed_browserstack_collection,
};
#[cfg(test)]
pub(crate) use cli::CiTarget;
pub use cli::MobileTarget;
pub(crate) use cli::PlotFixture;
pub(crate) use cli::{
    CheckOutputFormat, CiCommand, CiMergeSplitRunsArgs, Cli, Command, ConfigCommand,
    ContractErrorCategory, DevicePlatform, DevicesCommand, FixtureCommand, IosRunnerArg,
    IosSigningMethodArg, ProfileCommand, ReportCommand, SdkTarget, SummaryFormat,
};
#[cfg(test)]
pub(crate) use doctor::{
    DEFAULT_ANDROID_DOCTOR_RUST_TARGETS, WORKSPACE_MSRV, category_slug, parse_rust_version,
    render_check_results_json, rustc_version_meets_msrv,
};
pub(crate) use doctor::{
    PrereqCheck, cmd_check, cmd_config_validate, cmd_doctor, collect_issues,
    print_check_results_json, print_check_results_text,
};
use local_provider::{LocalProviderAdapter, LocalRunRequest};
use process_adapter::ToolCommand;
use project_layout::*;
pub(crate) use report_binding::RunEnvelopeIdentity;
use report_binding::bind_report_value;
use reporting::*;
use run_spec::*;

mod browserstack;
mod ci;
mod cli;
pub mod config;
mod doctor;
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

pub use ci::*;

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
            cmd_build(
                target,
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

pub(crate) fn run_ios_build(
    layout: &ResolvedProjectLayout,
    release: bool,
    dry_run: bool,
    ios_completion_timeout_secs: Option<u64>,
    ios_deployment_target: Option<&str>,
    ios_runner: Option<&str>,
) -> Result<(PathBuf, PathBuf)> {
    let ios_completion_timeout_secs =
        configured_ios_completion_timeout_secs(layout, ios_completion_timeout_secs);
    let ios_deployment_target = configured_ios_deployment_target(layout, ios_deployment_target)?;
    let ios_runner = configured_ios_runner(layout, &ios_deployment_target, ios_runner)?;
    let builder = ios_builder_for_layout(layout)
        .verbose(true)
        .dry_run(dry_run)
        .deployment_target(ios_deployment_target)
        .runner(Some(ios_runner))
        .benchmark_timeout_secs(ios_completion_timeout_secs)
        .crate_dir(&layout.crate_dir)
        .output_dir(&layout.output_dir);
    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Ios,
        profile,
        incremental: true,
        android_abis: None,
    };
    if let Some(timeout_secs) = ios_completion_timeout_secs {
        println!("Using iOS benchmark completion timeout: {timeout_secs}s");
    }
    let result = builder.build(&cfg)?;
    let header = layout
        .output_dir
        .join("ios/include")
        .join(format!("{}.h", layout.library_name));
    Ok((result.app_path, header))
}

fn package_ios_xcuitest_artifacts(
    layout: &ResolvedProjectLayout,
    spec: &RunSpec,
    identity: &RunEnvelopeIdentity,
    release: bool,
    ios_completion_timeout_secs: Option<u64>,
    ios_deployment_target: Option<&str>,
    ios_runner: Option<&str>,
) -> Result<IosXcuitestArtifacts> {
    let ios_completion_timeout_secs =
        configured_ios_completion_timeout_secs(layout, ios_completion_timeout_secs);
    let ios_deployment_target = configured_ios_deployment_target(layout, ios_deployment_target)?;
    let ios_runner = configured_ios_runner(layout, &ios_deployment_target, ios_runner)?;
    let builder = ios_builder_for_layout(layout)
        .verbose(true)
        .deployment_target(ios_deployment_target)
        .runner(Some(ios_runner))
        .benchmark_timeout_secs(ios_completion_timeout_secs)
        .crate_dir(&layout.crate_dir)
        .output_dir(&layout.output_dir);
    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Ios,
        profile,
        incremental: true,
        android_abis: None,
    };
    builder
        .build(&cfg)
        .context("Failed to build iOS xcframework before packaging")?;
    // `build()` refreshes generated XCUITest sources from the crate's detected
    // default. Re-embed after generation so the compiled test suite is bound to
    // the function requested for this run, not a stale scaffolding default.
    embed_spec_into_apps(&layout.output_dir, spec, identity)
        .context("Failed to bind generated iOS artifacts to the current bench spec")?;
    let app = builder
        .package_ipa("BenchRunner", mobench_sdk::builders::SigningMethod::AdHoc)
        .context("Failed to package iOS IPA for BrowserStack")?;
    let test_suite = builder
        .package_xcuitest("BenchRunner")
        .context("Failed to package iOS XCUITest runner for BrowserStack")?;
    Ok(IosXcuitestArtifacts { app, test_suite })
}

fn default_ios_xcuitest_artifacts(layout: &ResolvedProjectLayout) -> IosXcuitestArtifacts {
    IosXcuitestArtifacts {
        app: layout.output_dir.join("ios/BenchRunner.ipa"),
        test_suite: layout.output_dir.join("ios/BenchRunnerUITests.zip"),
    }
}

fn legacy_ios_xcuitest_artifacts(layout: &ResolvedProjectLayout) -> IosXcuitestArtifacts {
    IosXcuitestArtifacts {
        app: layout.project_root.join("target/ios/BenchRunner.ipa"),
        test_suite: layout
            .project_root
            .join("target/ios/BenchRunnerUITests.zip"),
    }
}

fn resolve_project_relative_path(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn uses_managed_ios_xcuitest_artifacts(
    layout: &ResolvedProjectLayout,
    artifacts: &IosXcuitestArtifacts,
) -> bool {
    let app = resolve_project_relative_path(&layout.project_root, &artifacts.app);
    let test_suite = resolve_project_relative_path(&layout.project_root, &artifacts.test_suite);

    [
        default_ios_xcuitest_artifacts(layout),
        legacy_ios_xcuitest_artifacts(layout),
    ]
    .into_iter()
    .any(|managed| app == managed.app && test_suite == managed.test_suite)
}

#[derive(Debug, Clone)]
struct ResolvedBrowserStack {
    username: String,
    access_key: String,
    project: Option<String>,
}

/// Represents artifacts validation error details for BrowserStack uploads.
#[derive(Debug)]
struct ArtifactValidationError {
    missing_artifacts: Vec<(String, PathBuf)>,
    target: MobileTarget,
}

impl ArtifactValidationError {
    fn format_error(&self) -> String {
        let mut msg = String::from("Missing required artifacts for BrowserStack run:\n\n");

        for (name, path) in &self.missing_artifacts {
            msg.push_str(&format!("  x {} not found at: {}\n", name, path.display()));
        }

        msg.push('\n');
        msg.push_str("To fix, run:\n");
        match self.target {
            MobileTarget::Android => {
                msg.push_str("  cargo mobench build --target android\n");
            }
            MobileTarget::Ios => {
                msg.push_str("  cargo mobench build --target ios\n");
                msg.push_str("  cargo mobench package-ipa --method adhoc\n");
                msg.push_str("  cargo mobench package-xcuitest\n");
            }
        }

        msg
    }
}

/// Validates that all required artifacts exist before attempting a BrowserStack upload.
///
/// This function checks for the presence of required files early to provide clear
/// error messages before starting any uploads.
///
/// # Arguments
/// * `target` - The target platform (Android or iOS)
/// * `apk` - For Android: path to the app APK
/// * `test_apk` - For Android: path to the test APK
/// * `ios_artifacts` - For iOS: the app and test suite paths
///
/// # Returns
/// * `Ok(())` if all artifacts exist
/// * `Err` with detailed message about missing artifacts and how to fix
fn validate_artifacts_for_browserstack(
    target: MobileTarget,
    apk: Option<&Path>,
    test_apk: Option<&Path>,
    ios_artifacts: Option<&IosXcuitestArtifacts>,
) -> Result<()> {
    let mut missing = Vec::new();

    match target {
        MobileTarget::Android => {
            if let Some(apk_path) = apk
                && !apk_path.exists()
            {
                missing.push(("Android APK".to_string(), apk_path.to_path_buf()));
            }
            if let Some(test_apk_path) = test_apk
                && !test_apk_path.exists()
            {
                missing.push(("Android test APK".to_string(), test_apk_path.to_path_buf()));
            }
        }
        MobileTarget::Ios => {
            if let Some(artifacts) = ios_artifacts {
                if !artifacts.app.exists() {
                    missing.push(("iOS app IPA".to_string(), artifacts.app.clone()));
                }
                if !artifacts.test_suite.exists() {
                    missing.push((
                        "iOS XCUITest runner".to_string(),
                        artifacts.test_suite.clone(),
                    ));
                }
            }
        }
    }

    if !missing.is_empty() {
        let error = ArtifactValidationError {
            missing_artifacts: missing,
            target,
        };
        bail!("{}", error.format_error());
    }

    Ok(())
}

/// Extracted benchmark result for a single device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedBenchmarkResult {
    /// Device name.
    pub device: String,
    /// Benchmark function name.
    pub function: String,
    /// Mean execution time in nanoseconds.
    pub mean_ns: u64,
    /// Number of samples collected.
    pub sample_count: usize,
    /// Standard deviation in nanoseconds (if calculable).
    pub std_dev_ns: Option<u64>,
    /// Minimum sample value in nanoseconds.
    pub min_ns: Option<u64>,
    /// Maximum sample value in nanoseconds.
    pub max_ns: Option<u64>,
}

/// Extract a unified summary from per-device benchmark results.
///
/// This function takes the raw benchmark results from BrowserStack and produces
/// a unified summary that's easier to work with programmatically.
pub fn extract_benchmark_summary(
    results: &HashMap<String, Vec<serde_json::Value>>,
) -> Vec<ExtractedBenchmarkResult> {
    let mut extracted = Vec::new();

    for (device, benchmarks) in results {
        for benchmark in benchmarks {
            let function = benchmark
                .get("function")
                .and_then(|f| f.as_str())
                .unwrap_or("unknown")
                .to_string();

            let producer_mean_ns = benchmark
                .get("mean_ns")
                .and_then(|m| m.as_u64())
                .unwrap_or(0);

            let samples = extract_samples(benchmark);

            let sample_count = samples.len();
            let statistics = Distribution::from_slice(&samples).sdk_v1_summary();
            let mean_ns = if samples.is_empty() {
                producer_mean_ns
            } else {
                statistics.mean_ns as u64
            };
            let min_ns = (!samples.is_empty()).then_some(statistics.min_ns);
            let max_ns = (!samples.is_empty()).then_some(statistics.max_ns);
            let std_dev_ns = (sample_count > 1).then_some(statistics.std_dev_ns as u64);

            extracted.push(ExtractedBenchmarkResult {
                device: device.clone(),
                function,
                mean_ns,
                sample_count,
                std_dev_ns,
                min_ns,
                max_ns,
            });
        }
    }

    extracted
}

fn trigger_browserstack_espresso(spec: &RunSpec, apk: &Path, test_apk: &Path) -> Result<RemoteRun> {
    // Validate artifacts exist before attempting upload
    validate_artifacts_for_browserstack(MobileTarget::Android, Some(apk), Some(test_apk), None)?;

    let creds = resolve_browserstack_credentials(spec.browserstack.as_ref())?;
    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username.clone(),
            access_key: creds.access_key.clone(),
        },
        creds.project.clone(),
    )?;

    let engine = mobench_provider::ProviderEngine::new(BrowserStackProviderAdapter::new(
        client,
        DEFAULT_BROWSERSTACK_FETCH_TIMEOUT_SECS,
        5,
    ));
    let request = BrowserStackRunRequest {
        devices: spec.devices.clone(),
        artifacts: BrowserStackArtifacts::Espresso {
            app: apk.to_path_buf(),
            test_suite: test_apk.to_path_buf(),
        },
    };
    let run = engine
        .start(&request, &mobench_process::global_cancellation_token())
        .map_err(|error| anyhow!("BrowserStack provider failed to start: {error}"))?
        .into_handle();

    // Print dashboard link early so users can monitor progress
    println!();
    println!("BrowserStack build started!");
    println!("  Build ID: {}", run.build_id);
    println!("  Devices:  {}", spec.devices.join(", "));
    println!(
        "  Dashboard: https://app-automate.browserstack.com/dashboard/v2/builds/{}",
        run.build_id
    );
    println!();
    println!("Waiting for results...");

    Ok(RemoteRun::Android {
        app_url: run.app_url,
        build_id: run.build_id,
    })
}

fn trigger_browserstack_xcuitest(
    spec: &RunSpec,
    artifacts: &IosXcuitestArtifacts,
) -> Result<RemoteRun> {
    // Validate artifacts exist before attempting upload
    validate_artifacts_for_browserstack(MobileTarget::Ios, None, None, Some(artifacts))?;

    let creds = resolve_browserstack_credentials(spec.browserstack.as_ref())?;
    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username.clone(),
            access_key: creds.access_key.clone(),
        },
        creds.project.clone(),
    )?;

    let engine = mobench_provider::ProviderEngine::new(BrowserStackProviderAdapter::new(
        client,
        DEFAULT_BROWSERSTACK_FETCH_TIMEOUT_SECS,
        5,
    ));
    let request = BrowserStackRunRequest {
        devices: spec.devices.clone(),
        artifacts: BrowserStackArtifacts::XcuiTest {
            app: artifacts.app.clone(),
            test_suite: artifacts.test_suite.clone(),
        },
    };
    let run = engine
        .start(&request, &mobench_process::global_cancellation_token())
        .map_err(|error| anyhow!("BrowserStack provider failed to start: {error}"))?
        .into_handle();

    // Print dashboard link early so users can monitor progress
    println!();
    println!("BrowserStack build started!");
    println!("  Build ID: {}", run.build_id);
    println!("  Devices:  {}", spec.devices.join(", "));
    println!(
        "  Dashboard: https://app-automate.browserstack.com/dashboard/v2/builds/{}",
        run.build_id
    );
    println!();
    println!("Waiting for results...");

    Ok(RemoteRun::Ios {
        app_url: run.app_url,
        test_suite_url: run
            .test_suite_url
            .context("BrowserStack XCUITest start omitted the test-suite URL")?,
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

    // Check what's missing and provide helpful error message
    let missing_username = username.as_deref().map(str::is_empty).unwrap_or(true);
    let missing_access_key = access_key.as_deref().map(str::is_empty).unwrap_or(true);

    if missing_username || missing_access_key {
        let error_msg =
            browserstack::format_credentials_error(missing_username, missing_access_key);
        bail!("{}", error_msg);
    }

    Ok(ResolvedBrowserStack {
        username: username.context("BrowserStack username resolved to None")?,
        access_key: access_key.context("BrowserStack access key resolved to None")?,
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

#[cfg(test)]
fn run_local_smoke(spec: &RunSpec) -> Result<Value> {
    println!("Running local smoke test for {}...", spec.function);

    let bench_spec = mobench_sdk::BenchSpec {
        name: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
    };

    let report =
        mobench_sdk::run_benchmark(bench_spec).map_err(|e| anyhow!("benchmark failed: {e}"))?;

    serde_json::to_value(&report).context("serializing benchmark report")
}

/// Validates that the benchmark function exists in the crate source.
///
/// This provides early feedback when a function name is misspelled or doesn't exist.
/// If validation fails, it warns but continues (the final validation happens on device).
pub(crate) fn validate_benchmark_function(
    layout: &ResolvedProjectLayout,
    function_name: &str,
) -> Result<()> {
    let benchmarks = discover_benchmarks_for_layout(layout)?;
    let found_any_benchmarks = !benchmarks.is_empty();
    let simple_name = function_name.split("::").last().unwrap_or(function_name);
    let found_function = benchmarks
        .iter()
        .any(|benchmark| benchmark == function_name)
        || benchmarks
            .iter()
            .any(|benchmark| benchmark.ends_with(&format!("::{}", simple_name)));

    if found_any_benchmarks && !found_function {
        // We found benchmarks but not the one requested - this is likely an error
        println!("=== Warning ===");
        println!(
            "  Benchmark function '{}' was not found in the source code.",
            function_name
        );
        println!("  Available benchmarks:");
        for bench in benchmarks {
            println!("    - {}", bench);
        }
        println!();
        println!("  The run will continue, but the benchmark may fail on the device.");
        println!("  Tip: Use 'cargo mobench list' to see all available benchmarks.");
        println!();
    } else if !found_any_benchmarks {
        // No benchmarks found at all - might be using direct dispatch
        println!("=== Note ===");
        println!(
            "  Could not validate benchmark function '{}' (no #[benchmark] functions found).",
            function_name
        );
        println!("  This is normal for projects using direct FFI dispatch (like sample-fns).");
        println!();
    } else {
        // Function validated successfully
        println!("Benchmark function '{}' validated.", function_name);
    }

    Ok(())
}

pub(crate) fn persist_mobile_spec(
    layout: &ResolvedProjectLayout,
    spec: &RunSpec,
    identity: &RunEnvelopeIdentity,
    release: bool,
) -> Result<()> {
    let root = &layout.project_root;
    let payload = json!({
        "schema_version": mobench_domain::REPORT_SCHEMA_V2,
        "run_id": identity.run_id,
        "nonce": identity.nonce,
        "logical_session_id": identity.logical_session_id,
        "function": spec.function,
        "function_id": spec.function,
        "producer": identity.producer,
        "iterations": spec.iterations,
        "warmup": spec.warmup,
        "android_benchmark_timeout_secs": spec.android_benchmark_timeout_secs,
        "android_heartbeat_interval_secs": spec.android_heartbeat_interval_secs,
    });
    let contents = serde_json::to_string_pretty(&payload)?;

    // Write to legacy mobile-spec locations for backward compatibility
    let legacy_targets = [
        root.join("target/mobile-spec/android/bench_spec.json"),
        root.join("target/mobile-spec/ios/bench_spec.json"),
    ];
    for path in legacy_targets {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {:?}", parent))?;
        }
        write_file(&path, contents.as_bytes())?;
    }

    // IMPORTANT: Also embed the spec directly into the mobile app bundles
    // This ensures the requested benchmark function is always used, even when
    // the app is run via BrowserStack where file paths are different.
    let mobench_output_dir = layout.output_dir.clone();
    let apps_exist =
        mobench_output_dir.join("android").exists() || mobench_output_dir.join("ios").exists();

    if let Err(e) = embed_spec_into_apps(&mobench_output_dir, spec, identity) {
        // Only warn if the apps don't exist yet - they'll be created during build
        if apps_exist {
            println!(
                "Warning: Failed to embed bench spec into app bundles: {}",
                e
            );
        }
    } else if apps_exist {
        println!("Embedded bench_spec.json in mobile app bundles");
    }

    // B3: Embed build metadata (bench_meta.json) for artifact correlation
    let profile = if release { "release" } else { "debug" };
    let target_str = match spec.target {
        MobileTarget::Android => "android",
        MobileTarget::Ios => "ios",
    };

    if let Err(e) = embed_meta_into_apps(&mobench_output_dir, spec, target_str, profile) {
        if apps_exist {
            println!(
                "Warning: Failed to embed bench meta into app bundles: {}",
                e
            );
        }
    } else if apps_exist {
        println!("Embedded bench_meta.json with build metadata");
    }

    Ok(())
}

/// Embeds the benchmark spec into Android assets and iOS bundle resources.
fn embed_spec_into_apps(
    output_dir: &Path,
    spec: &RunSpec,
    identity: &RunEnvelopeIdentity,
) -> Result<()> {
    #[derive(Serialize)]
    struct EmbeddedRunSpec {
        schema_version: &'static str,
        run_id: String,
        nonce: String,
        logical_session_id: String,
        function: String,
        function_id: String,
        producer: String,
        iterations: u32,
        warmup: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        android_benchmark_timeout_secs: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        android_heartbeat_interval_secs: Option<u64>,
    }

    let embedded_spec = EmbeddedRunSpec {
        schema_version: mobench_domain::REPORT_SCHEMA_V2,
        run_id: identity.run_id.clone(),
        nonce: identity.nonce.clone(),
        logical_session_id: identity.logical_session_id.clone(),
        function: spec.function.clone(),
        function_id: spec.function.clone(),
        producer: identity.producer.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
        android_benchmark_timeout_secs: spec.android_benchmark_timeout_secs,
        android_heartbeat_interval_secs: spec.android_heartbeat_interval_secs,
    };
    mobench_sdk::builders::embed_bench_spec(output_dir, &embedded_spec)
        .map_err(|e| anyhow!("Failed to embed bench spec: {}", e))
}

/// Embeds build metadata (bench_meta.json) into Android assets and iOS bundle resources.
fn embed_meta_into_apps(
    output_dir: &Path,
    spec: &RunSpec,
    target: &str,
    profile: &str,
) -> Result<()> {
    let embedded_spec = mobench_sdk::builders::EmbeddedBenchSpec {
        function: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
    };
    mobench_sdk::builders::embed_bench_meta(output_dir, &embedded_spec, target, profile)
        .map_err(|e| anyhow!("Failed to embed bench meta: {}", e))
}

pub(crate) fn run_android_build(
    layout: &ResolvedProjectLayout,
    _ndk_home: &str,
    release: bool,
    dry_run: bool,
) -> Result<mobench_sdk::BuildResult> {
    ensure_android_home();
    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Android,
        profile,
        incremental: true,
        android_abis: layout.android_abis.clone(),
    };
    let builder = android_builder_for_layout(layout)
        .verbose(true)
        .dry_run(dry_run)
        .crate_dir(&layout.crate_dir)
        .output_dir(&layout.output_dir);
    let result = builder.build(&cfg)?;
    Ok(result)
}

/// Ensure ANDROID_HOME is set, inferring it from ANDROID_NDK_HOME if necessary.
///
/// Gradle requires ANDROID_HOME to locate the SDK. Many developers only set
/// ANDROID_NDK_HOME (which is `$ANDROID_HOME/ndk/<version>`). This function
/// strips the `/ndk/<version>` suffix to derive ANDROID_HOME when it is missing.
fn ensure_android_home() {
    if std::env::var("ANDROID_HOME").is_ok() {
        return;
    }
    if let Ok(ndk_home) = std::env::var("ANDROID_NDK_HOME") {
        // ANDROID_NDK_HOME is typically $ANDROID_HOME/ndk/<version>
        let ndk_path = std::path::Path::new(&ndk_home);
        if let Some(ndk_dir) = ndk_path.parent()
            && ndk_dir.file_name().is_some_and(|n| n == "ndk")
            && let Some(sdk_root) = ndk_dir.parent()
        {
            eprintln!(
                "Inferred ANDROID_HOME={} from ANDROID_NDK_HOME",
                sdk_root.display()
            );
            // SAFETY: called early in single-threaded CLI init, before
            // any threads are spawned.
            unsafe { std::env::set_var("ANDROID_HOME", sdk_root) };
        }
    }
}

/// Load .env/.env.local from the repo root (best-effort, for commands that don't resolve a layout).
fn load_dotenv_global() {
    if let Ok(root) = repo_root() {
        let _ = dotenvy::from_path(root.join(".env"));
        let _ = dotenvy::from_path(root.join(".env.local"));
    }
}

pub(crate) fn load_dotenv_for_layout(layout: &ResolvedProjectLayout) {
    let mut directories = vec![layout.project_root.clone()];
    if let Some(config_path) = &layout.config_path
        && let Some(config_dir) = config_path.parent()
        && config_dir != layout.project_root
    {
        directories.push(config_dir.to_path_buf());
    }

    for dir in directories {
        let _ = dotenvy::from_path(dir.join(".env"));
        let _ = dotenvy::from_path(dir.join(".env.local"));
    }
}

pub(crate) fn repo_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("resolving repo root from current directory")?;
    if let Some(root) = find_repo_root(&cwd) {
        return Ok(root);
    }

    let compiled = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    if let Ok(path) = compiled.canonicalize() {
        if let Some(root) = find_repo_root(&path) {
            return Ok(root);
        }
        return Ok(path);
    }

    Ok(cwd)
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| is_repo_root(candidate))
        .map(|root| root.to_path_buf())
}

fn is_repo_root(candidate: &Path) -> bool {
    candidate.join("bench-mobile").join("Cargo.toml").is_file()
        || candidate
            .join("crates")
            .join("sample-fns")
            .join("Cargo.toml")
            .is_file()
}

fn ensure_can_write(path: &Path, overwrite: bool) -> Result<()> {
    if path.exists() && !overwrite {
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

/// Initialize a new benchmark project using `mobench-sdk`.
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

    let sdk_config = mobench_sdk::InitConfig {
        target: target.into(),
        project_name: project_name.clone(),
        output_dir: output_dir.clone(),
        generate_examples,
    };

    mobench_sdk::codegen::generate_project(&sdk_config).context("Failed to generate project")?;

    // Generate mobench.toml configuration file
    let mobench_toml_path = output_dir.join(config::CONFIG_FILE_NAME);
    if !mobench_toml_path.exists() {
        let toml_content = config::MobenchConfig::generate_starter_toml(&project_name);
        fs::write(&mobench_toml_path, toml_content)
            .with_context(|| format!("Failed to write {:?}", mobench_toml_path))?;
        println!("  Generated mobench.toml configuration file");
    }

    println!("\n[checkmark] Project initialized successfully!");
    println!("\nNext steps:");
    println!("  1. Add benchmark functions to your code with #[benchmark]");
    println!("  2. Edit mobench.toml to customize your project settings");
    println!("  3. Run 'cargo mobench build --target <platform>' to build");

    Ok(())
}

/// Build mobile artifacts using `mobench-sdk`.
#[allow(clippy::too_many_arguments)]
fn cmd_build(
    target: SdkTarget,
    release: bool,
    ios_completion_timeout_secs: Option<u64>,
    ios_deployment_target: Option<String>,
    ios_runner: Option<IosRunnerArg>,
    project_root: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    dry_run: bool,
    verbose: bool,
    progress: bool,
) -> Result<()> {
    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let effective_output_dir = output_dir.unwrap_or_else(|| layout.output_dir.clone());
    let ios_completion_timeout_secs =
        configured_ios_completion_timeout_secs(&layout, ios_completion_timeout_secs);
    let (ios_deployment_target, ios_runner) = if matches!(target, SdkTarget::Ios | SdkTarget::Both)
    {
        let deployment_target =
            configured_ios_deployment_target(&layout, ios_deployment_target.as_deref())?;
        let runner_name = ios_runner.map(ios_runner_arg_name);
        let runner = configured_ios_runner(&layout, &deployment_target, runner_name)?;
        (deployment_target, runner)
    } else {
        (
            mobench_sdk::codegen::IosDeploymentTarget::default_target(),
            mobench_sdk::codegen::IosRunner::Swiftui,
        )
    };

    // Progress mode: simplified output
    if progress {
        let build_config = mobench_sdk::BuildConfig {
            target: target.into(),
            profile: if release {
                mobench_sdk::BuildProfile::Release
            } else {
                mobench_sdk::BuildProfile::Debug
            },
            incremental: true,
            android_abis: layout.android_abis.clone(),
        };

        match target {
            SdkTarget::Android => {
                println!("[1/3] Building Rust library...");
                let builder = android_builder_for_layout(&layout)
                    .verbose(false)
                    .dry_run(dry_run)
                    .output_dir(&effective_output_dir)
                    .crate_dir(&layout.crate_dir);
                println!("[2/3] Building Android APK...");
                let result = builder.build(&build_config)?;
                println!("[3/3] Done!");
                if !dry_run {
                    println!("\n\u{2713} APK: {:?}", result.app_path);
                }
            }
            SdkTarget::Ios => {
                println!("[1/3] Building Rust library...");
                let builder = ios_builder_for_layout(&layout)
                    .verbose(false)
                    .dry_run(dry_run)
                    .deployment_target(ios_deployment_target.clone())
                    .runner(Some(ios_runner))
                    .benchmark_timeout_secs(ios_completion_timeout_secs)
                    .output_dir(&effective_output_dir)
                    .crate_dir(&layout.crate_dir);
                println!("[2/3] Building iOS xcframework...");
                let result = builder.build(&build_config)?;
                println!("[3/3] Done!");
                if !dry_run {
                    println!("\n\u{2713} Framework: {:?}", result.app_path);
                }
            }
            SdkTarget::Both => {
                println!("[1/5] Building Rust library for Android...");
                let android_builder = android_builder_for_layout(&layout)
                    .verbose(false)
                    .dry_run(dry_run)
                    .output_dir(&effective_output_dir)
                    .crate_dir(&layout.crate_dir);
                println!("[2/5] Building Android APK...");
                let android_result = android_builder.build(&build_config)?;

                println!("[3/5] Building Rust library for iOS...");
                let ios_builder = ios_builder_for_layout(&layout)
                    .verbose(false)
                    .dry_run(dry_run)
                    .deployment_target(ios_deployment_target.clone())
                    .runner(Some(ios_runner))
                    .benchmark_timeout_secs(ios_completion_timeout_secs)
                    .output_dir(&effective_output_dir)
                    .crate_dir(&layout.crate_dir);
                println!("[4/5] Building iOS xcframework...");
                let ios_result = ios_builder.build(&build_config)?;

                println!("[5/5] Done!");
                if !dry_run {
                    println!("\n\u{2713} APK: {:?}", android_result.app_path);
                    println!("\u{2713} Framework: {:?}", ios_result.app_path);
                }
            }
        }
        return Ok(());
    }

    // Normal (verbose) mode
    if let Some(config_path) = &layout.config_path {
        println!("Using config file: {:?}", config_path);
    }

    println!("Building mobile artifacts...");
    println!("  Target: {:?}", target);
    println!("  Profile: {}", if release { "release" } else { "debug" });
    if dry_run {
        println!("  Mode: dry-run (no changes will be made)");
    }
    if verbose {
        println!("  Verbose: enabled");
    }

    println!("  Output: {:?}", effective_output_dir);
    println!("  Project root: {:?}", layout.project_root);
    println!("  Crate: {:?}", layout.crate_dir);

    let build_config = mobench_sdk::BuildConfig {
        target: target.into(),
        profile: if release {
            mobench_sdk::BuildProfile::Release
        } else {
            mobench_sdk::BuildProfile::Debug
        },
        incremental: true,
        android_abis: layout.android_abis.clone(),
    };

    match target {
        SdkTarget::Android => {
            println!("\nBuilding for Android...");
            println!("  Building Rust library for Android targets...");
            let builder = android_builder_for_layout(&layout)
                .verbose(verbose)
                .dry_run(dry_run)
                .output_dir(&effective_output_dir)
                .crate_dir(&layout.crate_dir);
            let result = builder.build(&build_config)?;
            if !dry_run {
                println!("\u{2713} Built Android APK");
                println!("\n[checkmark] Android build completed!");
                println!("  APK: {:?}", result.app_path);
            }
        }
        SdkTarget::Ios => {
            println!("\nBuilding for iOS...");
            println!("  Building Rust library for iOS targets...");
            let builder = ios_builder_for_layout(&layout)
                .verbose(verbose)
                .dry_run(dry_run)
                .deployment_target(ios_deployment_target.clone())
                .runner(Some(ios_runner))
                .benchmark_timeout_secs(ios_completion_timeout_secs)
                .output_dir(&effective_output_dir)
                .crate_dir(&layout.crate_dir);
            let result = builder.build(&build_config)?;
            if !dry_run {
                println!("\u{2713} Built iOS xcframework");
                println!("\n[checkmark] iOS build completed!");
                println!("  Framework: {:?}", result.app_path);
            }
        }
        SdkTarget::Both => {
            // Build Android
            println!("\nBuilding for Android...");
            println!("  Building Rust library for Android targets...");
            let android_builder = android_builder_for_layout(&layout)
                .verbose(verbose)
                .dry_run(dry_run)
                .output_dir(&effective_output_dir)
                .crate_dir(&layout.crate_dir);
            let android_result = android_builder.build(&build_config)?;
            if !dry_run {
                println!("\u{2713} Built Android APK");
                println!("\n[checkmark] Android build completed!");
                println!("  APK: {:?}", android_result.app_path);
            }

            // Build iOS
            println!("\nBuilding for iOS...");
            println!("  Building Rust library for iOS targets...");
            let ios_builder = ios_builder_for_layout(&layout)
                .verbose(verbose)
                .dry_run(dry_run)
                .deployment_target(ios_deployment_target)
                .runner(Some(ios_runner))
                .benchmark_timeout_secs(ios_completion_timeout_secs)
                .output_dir(&effective_output_dir)
                .crate_dir(&layout.crate_dir);
            let ios_result = ios_builder.build(&build_config)?;
            if !dry_run {
                println!("\u{2713} Built iOS xcframework");
                println!("\n[checkmark] iOS build completed!");
                println!("  Framework: {:?}", ios_result.app_path);
            }
        }
    }

    if dry_run {
        println!("\n[dry-run] Build simulation completed. No changes were made.");
    }

    Ok(())
}

/// List all discovered benchmark functions
///
/// This uses source code scanning to find `#[benchmark]` functions, which works
/// without requiring a full build. It also falls back to the inventory registry
/// for any benchmarks that may be registered at runtime.
fn cmd_list(project_root: Option<PathBuf>, crate_path: Option<PathBuf>) -> Result<()> {
    println!("Discovering benchmark functions...\n");

    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let mut all_benchmarks = discover_benchmarks_for_layout(&layout)?;

    // Method 2: Inventory registry (for runtime-registered benchmarks)
    let registry_benchmarks = mobench_sdk::discover_benchmarks();
    for bench in registry_benchmarks {
        let name = bench.name.to_string();
        if !all_benchmarks.contains(&name) {
            all_benchmarks.push(name);
        }
    }

    all_benchmarks.sort();

    if all_benchmarks.is_empty() {
        println!("No benchmarks found.\n");
        println!("Resolved crate: {}", layout.crate_dir.display());
        println!("\nTo add benchmarks:");
        println!("  1. Add #[benchmark] attribute to functions");
        println!("  2. Make sure mobench-sdk is in your dependencies");
        println!("  3. Run 'cargo mobench list' again");
    } else {
        println!("Found {} benchmark(s):", all_benchmarks.len());
        for bench in &all_benchmarks {
            println!("  {}", bench);
        }
        println!();
        println!("Usage:");
        println!(
            "  cargo mobench run --target android --function {} --iterations 100",
            all_benchmarks
                .first()
                .map(|s| s.as_str())
                .unwrap_or("my_benchmark")
        );
    }

    Ok(())
}

/// Package iOS app as IPA for distribution or testing
fn cmd_package_ipa(
    scheme: &str,
    method: IosSigningMethodArg,
    project_root: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    println!("Packaging iOS app as IPA...");
    println!("  Scheme: {}", scheme);
    println!("  Method: {:?}", method);
    if let Some(ref dir) = output_dir {
        println!("  Output: {:?}", dir);
    }

    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let effective_output_dir = output_dir.unwrap_or_else(|| layout.output_dir.clone());
    let ios_deployment_target = configured_ios_deployment_target(&layout, None)?;
    let ios_runner = configured_ios_runner(&layout, &ios_deployment_target, None)?;

    let builder = ios_builder_for_layout(&layout)
        .verbose(true)
        .deployment_target(ios_deployment_target)
        .runner(Some(ios_runner))
        .crate_dir(&layout.crate_dir)
        .output_dir(&effective_output_dir);

    let signing_method: mobench_sdk::builders::SigningMethod = method.into();
    let ipa_path = builder
        .package_ipa(scheme, signing_method)
        .context("Failed to package IPA")?;

    println!("\n[checkmark] IPA packaged successfully!");
    println!("  Path: {:?}", ipa_path);
    println!("\nYou can now:");
    println!("  - Install on device: Use Xcode or ios-deploy");
    println!(
        "  - Test on BrowserStack: cargo mobench run --target ios --ios-app {:?}",
        ipa_path
    );

    Ok(())
}

/// Package XCUITest runner for BrowserStack testing
fn cmd_package_xcuitest(
    scheme: &str,
    project_root: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    println!("Packaging XCUITest runner...");
    println!("  Scheme: {}", scheme);
    if let Some(ref dir) = output_dir {
        println!("  Output: {:?}", dir);
    }

    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let effective_output_dir = output_dir.unwrap_or_else(|| layout.output_dir.clone());
    let ios_deployment_target = configured_ios_deployment_target(&layout, None)?;
    let ios_runner = configured_ios_runner(&layout, &ios_deployment_target, None)?;

    let builder = ios_builder_for_layout(&layout)
        .verbose(true)
        .deployment_target(ios_deployment_target)
        .runner(Some(ios_runner))
        .crate_dir(&layout.crate_dir)
        .output_dir(&effective_output_dir);

    let zip_path = builder
        .package_xcuitest(scheme)
        .context("Failed to package XCUITest runner")?;

    println!("\n[checkmark] XCUITest runner packaged successfully!");
    println!("  Path: {:?}", zip_path);
    println!("\nYou can now:");
    println!(
        "  - Test on BrowserStack: cargo mobench run --target ios --ios-test-suite {:?}",
        zip_path
    );

    Ok(())
}

/// Verify benchmark setup: registry, spec, artifacts, and optional smoke test
#[allow(clippy::too_many_arguments)]
fn cmd_verify(
    project_root: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    target: Option<SdkTarget>,
    spec_path: Option<PathBuf>,
    check_artifacts: bool,
    smoke_test: bool,
    function: Option<String>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    println!("Verifying benchmark setup...\n");

    let layout = resolve_project_layout(ProjectLayoutOptions {
        start_dir: None,
        project_root: project_root.as_deref(),
        crate_path: crate_path.as_deref(),
        config_path: None,
    })?;
    let resolved_benchmarks = discover_benchmarks_for_layout(&layout)?;
    let effective_output_dir = output_dir.unwrap_or_else(|| layout.output_dir.clone());

    let mut checks_passed = 0;
    let mut checks_failed = 0;
    let mut warnings = 0;

    // 1. Check benchmark registry
    print!("  [1/4] Checking benchmark registry... ");
    let registry_benchmarks = mobench_sdk::discover_benchmarks();
    if resolved_benchmarks.is_empty() && registry_benchmarks.is_empty() {
        println!("WARNING");
        println!("        No benchmarks found in registry.");
        println!("        This may be expected if benchmarks are in a separate crate.");
        println!(
            "        Tip: Add #[benchmark] attribute to functions and ensure mobench-sdk is linked."
        );
        warnings += 1;
    } else {
        let total = resolved_benchmarks.len().max(registry_benchmarks.len());
        println!("OK ({} benchmark(s) found)", total);
        for bench in &resolved_benchmarks {
            println!("        - {}", bench);
        }
        if resolved_benchmarks.is_empty() {
            for bench in &registry_benchmarks {
                println!("        - {}", bench.name);
            }
        }
        checks_passed += 1;
    }

    // 2. Validate spec file if provided
    print!("  [2/4] Checking spec file... ");
    if let Some(ref path) = spec_path {
        match validate_spec_file(path) {
            Ok(spec) => {
                println!("OK");
                println!("        Function: {}", spec.name);
                println!("        Iterations: {}", spec.iterations);
                println!("        Warmup: {}", spec.warmup);
                checks_passed += 1;
            }
            Err(e) => {
                println!("FAILED");
                println!("        Error: {}", e);
                checks_failed += 1;
            }
        }
    } else {
        // Try default locations
        let default_paths = [
            effective_output_dir.join("android/app/src/main/assets/bench_spec.json"),
            effective_output_dir.join("ios/BenchRunner/BenchRunner/bench_spec.json"),
            layout
                .project_root
                .join("target/mobile-spec/android/bench_spec.json"),
            layout
                .project_root
                .join("target/mobile-spec/ios/bench_spec.json"),
        ];

        let mut found_any = false;
        for path in &default_paths {
            if path.exists() {
                if !found_any {
                    println!("OK (found at default locations)");
                    found_any = true;
                }
                match validate_spec_file(path) {
                    Ok(spec) => {
                        println!("        {:?}", path);
                        println!(
                            "          Function: {}, Iterations: {}, Warmup: {}",
                            spec.name, spec.iterations, spec.warmup
                        );
                    }
                    Err(e) => {
                        println!("        {:?} - INVALID: {}", path, e);
                    }
                }
            }
        }
        if found_any {
            checks_passed += 1;
        } else {
            println!("SKIPPED (no spec file found, use --spec-path to specify)");
            warnings += 1;
        }
    }

    // 3. Check artifacts if requested
    print!("  [3/4] Checking build artifacts... ");
    if check_artifacts {
        let mut artifacts_ok = true;
        let mut artifact_details = Vec::new();

        if let Some(ref t) = target {
            match t {
                SdkTarget::Android | SdkTarget::Both => {
                    let apk_path = effective_output_dir
                        .join("android/app/build/outputs/apk/debug/app-debug.apk");
                    let apk_release = effective_output_dir
                        .join("android/app/build/outputs/apk/release/app-release-unsigned.apk");
                    if apk_path.exists() {
                        artifact_details.push(format!("Android APK (debug): {:?}", apk_path));
                    } else if apk_release.exists() {
                        artifact_details.push(format!("Android APK (release): {:?}", apk_release));
                    } else {
                        artifact_details.push("Android APK: NOT FOUND".to_string());
                        artifacts_ok = false;
                    }

                    // Check JNI libs
                    let jni_base = effective_output_dir.join("android/app/src/main/jniLibs");
                    let abis = configured_android_abis(&layout);
                    for abi in abis {
                        let lib_path = jni_base
                            .join(&abi)
                            .join(format!("lib{}.so", layout.library_name));
                        if lib_path.exists() {
                            artifact_details.push(format!("JNI lib ({}): OK", abi));
                        }
                    }
                }
                SdkTarget::Ios => {}
            }

            match t {
                SdkTarget::Ios | SdkTarget::Both => {
                    let xcframework = effective_output_dir
                        .join("ios")
                        .join(format!("{}.xcframework", layout.library_name));
                    if xcframework.exists() {
                        artifact_details.push(format!("iOS xcframework: {:?}", xcframework));
                    } else {
                        artifact_details.push("iOS xcframework: NOT FOUND".to_string());
                        artifacts_ok = false;
                    }

                    let ipa_path = effective_output_dir.join("ios/BenchRunner.ipa");
                    if ipa_path.exists() {
                        artifact_details.push(format!("iOS IPA: {:?}", ipa_path));
                    }

                    let xcuitest_path = effective_output_dir.join("ios/BenchRunnerUITests.zip");
                    if xcuitest_path.exists() {
                        artifact_details.push(format!("XCUITest runner: {:?}", xcuitest_path));
                    }
                }
                SdkTarget::Android => {}
            }
        } else {
            // Check both platforms by default
            let android_apk =
                effective_output_dir.join("android/app/build/outputs/apk/debug/app-debug.apk");
            let ios_xcframework = effective_output_dir
                .join("ios")
                .join(format!("{}.xcframework", layout.library_name));

            if android_apk.exists() {
                artifact_details.push(format!("Android APK: {:?}", android_apk));
            }
            if ios_xcframework.exists() {
                artifact_details.push(format!("iOS xcframework: {:?}", ios_xcframework));
            }

            if artifact_details.is_empty() {
                artifacts_ok = false;
                artifact_details
                    .push("No artifacts found. Run 'cargo mobench build' first.".to_string());
            }
        }

        if artifacts_ok {
            println!("OK");
            checks_passed += 1;
        } else {
            println!("FAILED");
            checks_failed += 1;
        }
        for detail in &artifact_details {
            println!("        {}", detail);
        }
    } else {
        println!("SKIPPED (use --check-artifacts to enable)");
    }

    // 4. Run smoke test if requested
    print!("  [4/4] Running smoke test... ");
    if smoke_test {
        if let Err(err) = ensure_verify_smoke_test_supported(&layout) {
            println!("SKIPPED");
            println!("        {}", err);
            warnings += 1;
        } else if let Some(ref func) = function {
            match run_verify_smoke_test(func) {
                Ok(report) => {
                    println!("OK");
                    let samples = report.samples.len();
                    let mean_ns = verify_report_mean_ns(&report);
                    println!("        Function: {}", func);
                    println!("        Samples: {}", samples);
                    println!(
                        "        Mean: {} ns ({:.3} ms)",
                        mean_ns,
                        mean_ns as f64 / 1_000_000.0
                    );
                    checks_passed += 1;
                }
                Err(e) => {
                    println!("FAILED");
                    println!("        Error: {}", e);
                    checks_failed += 1;
                }
            }
        } else if let Some(func) = layout
            .default_function
            .as_ref()
            .or_else(|| resolved_benchmarks.first())
        {
            match run_verify_smoke_test(func) {
                Ok(report) => {
                    println!("OK");
                    let samples = report.samples.len();
                    let mean_ns = verify_report_mean_ns(&report);
                    println!("        Function: {} (auto-selected)", func);
                    println!("        Samples: {}", samples);
                    println!(
                        "        Mean: {} ns ({:.3} ms)",
                        mean_ns,
                        mean_ns as f64 / 1_000_000.0
                    );
                    checks_passed += 1;
                }
                Err(e) => {
                    println!("FAILED");
                    println!("        Error: {}", e);
                    checks_failed += 1;
                }
            }
        } else {
            println!("SKIPPED (no benchmark function available)");
            println!(
                "        Tip: Use --function to specify a function, or add benchmarks with #[benchmark]"
            );
            warnings += 1;
        }
    } else {
        println!("SKIPPED (use --smoke-test to enable)");
    }

    // Print summary
    println!("\n----------------------------------------");
    println!("Verification Summary:");
    println!("  Passed:   {}", checks_passed);
    println!("  Failed:   {}", checks_failed);
    println!("  Warnings: {}", warnings);

    if checks_failed > 0 {
        println!("\n[X] Verification failed with {} error(s)", checks_failed);
        bail!("Verification failed");
    } else if warnings > 0 {
        println!("\n[!] Verification completed with {} warning(s)", warnings);
    } else {
        println!("\n[checkmark] All checks passed!");
    }

    Ok(())
}

fn verify_report_mean_ns(report: &mobench_sdk::timing::BenchReport) -> u64 {
    let durations = report
        .samples
        .iter()
        .map(|sample| sample.duration_ns)
        .collect::<Vec<_>>();
    Distribution::from_slice(&durations)
        .cli_v1_summary()
        .map_or(0, |summary| summary.mean_ns)
}

/// Validate a bench_spec.json file
///
/// Handles both "name" and "function" field names for compatibility
/// with different spec file formats.
fn validate_spec_file(path: &Path) -> Result<mobench_sdk::BenchSpec> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading spec file {:?}", path))?;

    // Try parsing directly first (standard BenchSpec format with "name" field)
    if let Ok(spec) = serde_json::from_str::<mobench_sdk::BenchSpec>(&contents) {
        // Validate spec fields
        if spec.name.trim().is_empty() {
            bail!("spec.name is empty");
        }
        if spec.iterations == 0 {
            bail!("spec.iterations must be > 0");
        }
        return Ok(spec);
    }

    // Fall back to generic Value parsing for "function" field format
    // (used by persist_mobile_spec and some older formats)
    let value: Value =
        serde_json::from_str(&contents).with_context(|| format!("parsing spec file {:?}", path))?;

    // Extract name from either "name" or "function" field
    let name = value
        .get("name")
        .or_else(|| value.get("function"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("spec must have 'name' or 'function' field"))?
        .to_string();

    let iterations = value
        .get("iterations")
        .map(|value| {
            json_value_to_u32(value)
                .ok_or_else(|| anyhow!("spec.iterations must be an unsigned 32-bit integer"))
        })
        .transpose()?
        .unwrap_or(100);

    let warmup = value
        .get("warmup")
        .map(|value| {
            json_value_to_u32(value)
                .ok_or_else(|| anyhow!("spec.warmup must be an unsigned 32-bit integer"))
        })
        .transpose()?
        .unwrap_or(10);

    // Validate
    if name.trim().is_empty() {
        bail!("spec.name/function is empty");
    }
    if iterations == 0 {
        bail!("spec.iterations must be > 0");
    }

    mobench_sdk::BenchSpec::new(name, iterations, warmup).map_err(anyhow::Error::from)
}

/// Run a minimal smoke test for verification
fn run_verify_smoke_test(function: &str) -> Result<mobench_sdk::RunnerReport> {
    let spec = mobench_sdk::BenchSpec {
        name: function.to_string(),
        iterations: 3, // Minimal iterations for smoke test
        warmup: 1,
    };

    mobench_sdk::run_benchmark(spec).map_err(|e| anyhow!("smoke test failed: {}", e))
}

/// Display summary statistics from a benchmark report JSON file
fn cmd_summary(report_path: &Path, format: Option<SummaryFormat>) -> Result<()> {
    let format = format.unwrap_or(SummaryFormat::Text);

    // Try to load the report in various formats
    let contents = fs::read_to_string(report_path)
        .with_context(|| format!("reading report file {:?}", report_path))?;

    let value: Value = serde_json::from_str(&contents)
        .with_context(|| format!("parsing report file {:?}", report_path))?;

    // Extract summary information
    let summary_data = extract_summary_data(&value)?;

    match format {
        SummaryFormat::Text => print_summary_text(&summary_data),
        SummaryFormat::Json => print_summary_json(&summary_data)?,
        SummaryFormat::Csv => print_summary_csv(&summary_data),
    }

    Ok(())
}

/// Summary data extracted from various report formats
#[derive(Debug, Serialize)]
struct SummaryData {
    source_file: String,
    function: Option<String>,
    device: Option<String>,
    os_version: Option<String>,
    sample_count: usize,
    mean_ns: Option<u64>,
    median_ns: Option<u64>,
    min_ns: Option<u64>,
    max_ns: Option<u64>,
    p95_ns: Option<u64>,
    iterations: Option<u32>,
    warmup: Option<u32>,
}

/// Extract summary data from various report formats
fn extract_summary_data(value: &Value) -> Result<Vec<SummaryData>> {
    let mut results = Vec::new();

    // Check if this is a RunSummary format (from `mobench run`)
    if value.get("summary").is_some() {
        let summary = &value["summary"];
        let function = summary
            .get("function")
            .and_then(|f| f.as_str())
            .map(String::from);
        let iterations = summary.get("iterations").and_then(json_value_to_u32);
        let warmup = summary.get("warmup").and_then(json_value_to_u32);

        if let Some(device_summaries) = summary.get("device_summaries").and_then(|d| d.as_array()) {
            for device_summary in device_summaries {
                let device = device_summary
                    .get("device")
                    .and_then(|d| d.as_str())
                    .map(String::from);

                if let Some(benchmarks) =
                    device_summary.get("benchmarks").and_then(|b| b.as_array())
                {
                    for bench in benchmarks {
                        let bench_function = bench
                            .get("function")
                            .and_then(|f| f.as_str())
                            .map(String::from);
                        results.push(SummaryData {
                            source_file: "RunSummary".to_string(),
                            function: bench_function.or_else(|| function.clone()),
                            device: device.clone(),
                            os_version: None, // RunSummary doesn't include OS version directly
                            sample_count: bench.get("samples").and_then(|s| s.as_u64()).unwrap_or(0)
                                as usize,
                            mean_ns: bench.get("mean_ns").and_then(|m| m.as_u64()),
                            median_ns: bench.get("median_ns").and_then(|m| m.as_u64()),
                            min_ns: bench.get("min_ns").and_then(|m| m.as_u64()),
                            max_ns: bench.get("max_ns").and_then(|m| m.as_u64()),
                            p95_ns: bench.get("p95_ns").and_then(|p| p.as_u64()),
                            iterations,
                            warmup,
                        });
                    }
                }
            }
        }
    }

    // Check if this is a BenchReport format (direct timing output)
    if let Some(spec) = value.get("spec") {
        let samples = extract_samples(value);
        let stats = compute_sample_stats(&samples);

        results.push(SummaryData {
            source_file: "BenchReport".to_string(),
            function: spec.get("name").and_then(|n| n.as_str()).map(String::from),
            device: Some("local".to_string()),
            os_version: None,
            sample_count: samples.len(),
            mean_ns: stats.as_ref().map(|s| s.mean_ns),
            median_ns: stats.as_ref().map(|s| s.median_ns),
            min_ns: stats.as_ref().map(|s| s.min_ns),
            max_ns: stats.as_ref().map(|s| s.max_ns),
            p95_ns: stats.as_ref().map(|s| s.p95_ns),
            iterations: spec.get("iterations").and_then(json_value_to_u32),
            warmup: spec.get("warmup").and_then(json_value_to_u32),
        });
    }

    // Check if this is benchmark_results format (from BrowserStack fetch)
    if let Some(benchmark_results) = value.get("benchmark_results").and_then(|b| b.as_object()) {
        for (device, entries) in benchmark_results {
            if let Some(entries) = entries.as_array() {
                for entry in entries {
                    let samples = extract_samples(entry);
                    let stats = compute_sample_stats(&samples);

                    results.push(SummaryData {
                        source_file: "BrowserStack".to_string(),
                        function: entry
                            .get("function")
                            .and_then(|f| f.as_str())
                            .map(String::from),
                        device: Some(device.clone()),
                        os_version: entry
                            .get("os_version")
                            .and_then(|o| o.as_str())
                            .map(String::from),
                        sample_count: samples.len(),
                        mean_ns: entry
                            .get("mean_ns")
                            .and_then(|m| m.as_u64())
                            .or_else(|| stats.as_ref().map(|s| s.mean_ns)),
                        median_ns: stats.as_ref().map(|s| s.median_ns),
                        min_ns: stats.as_ref().map(|s| s.min_ns),
                        max_ns: stats.as_ref().map(|s| s.max_ns),
                        p95_ns: stats.as_ref().map(|s| s.p95_ns),
                        iterations: None,
                        warmup: None,
                    });
                }
            }
        }
    }

    // Check if this is a session bench-report.json format
    if value.get("samples").is_some() && value.get("spec").is_none() {
        // Direct samples array without spec wrapper
        let samples = extract_samples(value);
        let stats = compute_sample_stats(&samples);

        results.push(SummaryData {
            source_file: "SessionReport".to_string(),
            function: value
                .get("function")
                .and_then(|f| f.as_str())
                .map(String::from),
            device: value
                .get("device")
                .and_then(|d| d.as_str())
                .map(String::from),
            os_version: value
                .get("os_version")
                .and_then(|o| o.as_str())
                .map(String::from),
            sample_count: samples.len(),
            mean_ns: value
                .get("mean_ns")
                .and_then(|m| m.as_u64())
                .or_else(|| stats.as_ref().map(|s| s.mean_ns)),
            median_ns: stats.as_ref().map(|s| s.median_ns),
            min_ns: stats.as_ref().map(|s| s.min_ns),
            max_ns: stats.as_ref().map(|s| s.max_ns),
            p95_ns: stats.as_ref().map(|s| s.p95_ns),
            iterations: value.get("iterations").and_then(json_value_to_u32),
            warmup: value.get("warmup").and_then(json_value_to_u32),
        });
    }

    if results.is_empty() {
        bail!("Could not extract summary data from report. Unrecognized format.");
    }

    Ok(results)
}

/// Print summary in text format
fn print_summary_text(data: &[SummaryData]) {
    println!("Benchmark Summary");
    println!("=================\n");

    for (idx, entry) in data.iter().enumerate() {
        if data.len() > 1 {
            println!("--- Entry {} ---", idx + 1);
        }

        if let Some(ref func) = entry.function {
            println!("Function:     {}", func);
        }
        if let Some(ref device) = entry.device {
            println!("Device:       {}", device);
        }
        if let Some(ref os) = entry.os_version {
            println!("OS Version:   {}", os);
        }
        println!("Sample Count: {}", entry.sample_count);
        println!();

        println!("Statistics (nanoseconds):");
        println!(
            "  Mean:   {}",
            entry
                .mean_ns
                .map(|v| format!("{} ({:.3} ms)", v, v as f64 / 1_000_000.0))
                .unwrap_or_else(|| "-".to_string())
        );
        println!(
            "  Median: {}",
            entry
                .median_ns
                .map(|v| format!("{} ({:.3} ms)", v, v as f64 / 1_000_000.0))
                .unwrap_or_else(|| "-".to_string())
        );
        println!(
            "  Min:    {}",
            entry
                .min_ns
                .map(|v| format!("{} ({:.3} ms)", v, v as f64 / 1_000_000.0))
                .unwrap_or_else(|| "-".to_string())
        );
        println!(
            "  Max:    {}",
            entry
                .max_ns
                .map(|v| format!("{} ({:.3} ms)", v, v as f64 / 1_000_000.0))
                .unwrap_or_else(|| "-".to_string())
        );
        println!(
            "  P95:    {}",
            entry
                .p95_ns
                .map(|v| format!("{} ({:.3} ms)", v, v as f64 / 1_000_000.0))
                .unwrap_or_else(|| "-".to_string())
        );

        if entry.iterations.is_some() || entry.warmup.is_some() {
            println!();
            println!("Configuration:");
            if let Some(iter) = entry.iterations {
                println!("  Iterations: {}", iter);
            }
            if let Some(warm) = entry.warmup {
                println!("  Warmup:     {}", warm);
            }
        }

        if idx < data.len() - 1 {
            println!();
        }
    }
}

/// Print summary in JSON format
fn print_summary_json(data: &[SummaryData]) -> Result<()> {
    let json = serde_json::to_string_pretty(data)?;
    println!("{}", json);
    Ok(())
}

/// Print summary in CSV format
fn render_summary_data_csv(data: &[SummaryData]) -> String {
    let mut output = String::from(
        "function,device,os_version,sample_count,mean_ns,median_ns,min_ns,max_ns,p95_ns,iterations,warmup\n",
    );
    for entry in data {
        let _ = writeln!(
            output,
            "{},{},{},{},{},{},{},{},{},{},{}",
            csv_field(entry.function.as_deref().unwrap_or("")),
            csv_field(entry.device.as_deref().unwrap_or("")),
            csv_field(entry.os_version.as_deref().unwrap_or("")),
            entry.sample_count,
            entry.mean_ns.map(|v| v.to_string()).unwrap_or_default(),
            entry.median_ns.map(|v| v.to_string()).unwrap_or_default(),
            entry.min_ns.map(|v| v.to_string()).unwrap_or_default(),
            entry.max_ns.map(|v| v.to_string()).unwrap_or_default(),
            entry.p95_ns.map(|v| v.to_string()).unwrap_or_default(),
            entry.iterations.map(|v| v.to_string()).unwrap_or_default(),
            entry.warmup.map(|v| v.to_string()).unwrap_or_default(),
        );
    }
    output
}

fn print_summary_csv(data: &[SummaryData]) {
    print!("{}", render_summary_data_csv(data));
}

/// List available BrowserStack devices and optionally validate device specs.
fn cmd_devices(
    platform: Option<DevicePlatform>,
    output_json: bool,
    validate: Vec<String>,
) -> Result<()> {
    // Try to get credentials, but provide helpful error if missing
    let creds = match resolve_browserstack_credentials(None) {
        Ok(creds) => creds,
        Err(_) => {
            // Check what's missing and provide helpful guidance
            let username = env::var("BROWSERSTACK_USERNAME").ok();
            let access_key = env::var("BROWSERSTACK_ACCESS_KEY").ok();

            let missing_username = username.is_none() || username.as_deref() == Some("");
            let missing_access_key = access_key.is_none() || access_key.as_deref() == Some("");

            let error_msg =
                browserstack::format_credentials_error(missing_username, missing_access_key);
            bail!("{}", error_msg);
        }
    };

    let client = BrowserStackClient::new(
        BrowserStackAuth {
            username: creds.username,
            access_key: creds.access_key,
        },
        creds.project,
    )?;

    // If validating devices, do that and exit
    if !validate.is_empty() {
        let platform_str = platform.map(|p| match p {
            DevicePlatform::Android => "android",
            DevicePlatform::Ios => "ios",
        });

        let validation = client.validate_devices(&validate, platform_str)?;

        if output_json {
            let output = json!({
                "valid": validation.valid,
                "invalid": validation.invalid.iter().map(|e| {
                    json!({
                        "spec": e.spec,
                        "reason": e.reason,
                        "suggestions": e.suggestions
                    })
                }).collect::<Vec<_>>()
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            if !validation.valid.is_empty() {
                println!("Valid devices ({}):", validation.valid.len());
                for device in &validation.valid {
                    println!("  [OK] {}", device);
                }
            }

            if !validation.invalid.is_empty() {
                if !validation.valid.is_empty() {
                    println!();
                }
                println!("Invalid devices ({}):", validation.invalid.len());
                for error in &validation.invalid {
                    println!("  [ERROR] {}: {}", error.spec, error.reason);
                    if !error.suggestions.is_empty() {
                        println!("          Suggestions:");
                        for suggestion in &error.suggestions {
                            println!("            - {}", suggestion);
                        }
                    }
                }
            }
        }

        // Exit with error if any devices were invalid
        if !validation.invalid.is_empty() {
            bail!(
                "{} of {} device specs are invalid",
                validation.invalid.len(),
                validate.len()
            );
        }

        return Ok(());
    }

    // List devices
    println!("Fetching available BrowserStack devices...\n");

    let devices = match platform {
        Some(DevicePlatform::Android) => client.list_espresso_devices()?,
        Some(DevicePlatform::Ios) => client.list_xcuitest_devices()?,
        None => client.list_all_devices()?,
    };

    if devices.is_empty() {
        println!("No devices found.");
        return Ok(());
    }

    if output_json {
        println!("{}", serde_json::to_string_pretty(&devices)?);
        return Ok(());
    }

    // Group devices by OS
    let mut android_devices: Vec<_> = devices.iter().filter(|d| d.os == "android").collect();
    let mut ios_devices: Vec<_> = devices.iter().filter(|d| d.os == "ios").collect();

    // Sort by device name, then OS version (descending)
    android_devices.sort_by(|a, b| {
        a.device.cmp(&b.device).then_with(|| {
            // Try to compare versions numerically
            let av: f64 = a.os_version.parse().unwrap_or(0.0);
            let bv: f64 = b.os_version.parse().unwrap_or(0.0);
            bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    ios_devices.sort_by(|a, b| {
        a.device.cmp(&b.device).then_with(|| {
            let av: f64 = a.os_version.parse().unwrap_or(0.0);
            let bv: f64 = b.os_version.parse().unwrap_or(0.0);
            bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    if !android_devices.is_empty() {
        println!("Android Devices ({}):", android_devices.len());
        println!("{:-<60}", "");
        for device in &android_devices {
            println!("  {:40} OS {}", device.device, device.os_version);
            println!("    --devices \"{}\"", device.identifier());
        }
        println!();
    }

    if !ios_devices.is_empty() {
        println!("iOS Devices ({}):", ios_devices.len());
        println!("{:-<60}", "");
        for device in &ios_devices {
            println!("  {:40} iOS {}", device.device, device.os_version);
            println!("    --devices \"{}\"", device.identifier());
        }
        println!();
    }

    println!("Total: {} devices available", devices.len());
    println!("\nUsage:");
    println!("  cargo mobench run --target android --devices \"Google Pixel 7-13.0\" ...");
    println!("  cargo mobench run --target ios --devices \"iPhone 14-16\" ...");

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ResolvedMatrixDevice {
    pub(crate) name: String,
    pub(crate) os: String,
    pub(crate) os_version: String,
    pub(crate) identifier: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedDeviceProfile {
    pub(crate) profile: String,
    pub(crate) source: String,
    pub(crate) devices: Vec<ResolvedMatrixDevice>,
}

/// Built-in device profiles so `devices resolve` works without a YAML file.
fn builtin_device_for_profile(
    platform: DevicePlatform,
    profile: &str,
) -> Option<ResolvedMatrixDevice> {
    let (name, os, os_version) = match (platform, profile) {
        (DevicePlatform::Ios, "low-spec") => ("iPhone SE 2020", "ios", "16"),
        (DevicePlatform::Ios, "mid-spec") => ("iPhone 14", "ios", "16"),
        (DevicePlatform::Ios, "high-spec") => ("iPhone 16 Pro", "ios", "18"),
        (DevicePlatform::Android, "low-spec") => ("Motorola Moto G9 Play", "android", "10.0"),
        (DevicePlatform::Android, "mid-spec") => ("Google Pixel 7", "android", "13.0"),
        (DevicePlatform::Android, "high-spec") => ("Samsung Galaxy S24", "android", "14.0"),
        _ => return None,
    };
    Some(ResolvedMatrixDevice {
        identifier: format!("{name}-{os_version}"),
        name: name.to_string(),
        os: os.to_string(),
        os_version: os_version.to_string(),
        tags: vec![profile.to_string()],
    })
}

pub(crate) fn resolve_devices_for_profile(
    platform: DevicePlatform,
    profile: Option<&str>,
    config_path: Option<&Path>,
    device_matrix_path: Option<&Path>,
) -> Result<ResolvedDeviceProfile> {
    let profile_str = profile
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");

    let (devices, source) = match resolve_matrix_for_cli(config_path, device_matrix_path) {
        Ok((matrix_path, config_tags)) => {
            let matrix = load_device_matrix(&matrix_path).with_context(|| {
                format!(
                    "config_error: failed to parse device matrix at {}",
                    matrix_path.display()
                )
            })?;
            let selected_tags = if profile.is_some() {
                vec![profile_str.to_string()]
            } else {
                config_tags
                    .filter(|tags| !tags.is_empty())
                    .unwrap_or_else(|| vec!["default".to_string()])
            };
            let devices = resolve_devices_from_matrix(matrix.devices, platform, &selected_tags)?;
            (devices, format!("matrix:{}", matrix_path.display()))
        }
        Err(_) => {
            if let Some(device) = builtin_device_for_profile(platform, profile_str) {
                (vec![device], "builtin".to_string())
            } else {
                bail!(
                    "No device matrix found and '{}' is not a built-in profile. \
                         Built-in profiles: low-spec, mid-spec, high-spec",
                    profile_str
                );
            }
        }
    };

    Ok(ResolvedDeviceProfile {
        profile: profile_str.to_string(),
        source,
        devices,
    })
}

fn cmd_devices_resolve(
    platform: DevicePlatform,
    profile: Option<String>,
    config_path: Option<&Path>,
    device_matrix_path: Option<&Path>,
    format: CheckOutputFormat,
) -> Result<()> {
    let resolved_profile = resolve_devices_for_profile(
        platform,
        profile.as_deref(),
        config_path,
        device_matrix_path,
    )?;
    let profile_str = resolved_profile.profile.as_str();
    let resolved = &resolved_profile.devices;
    let source = resolved_profile.source.as_str();

    match format {
        CheckOutputFormat::Text => {
            for device in resolved {
                println!("{}", device.identifier);
            }
        }
        CheckOutputFormat::Json => {
            let first: Option<&ResolvedMatrixDevice> = resolved.first();
            let output = json!({
                "platform": match platform {
                    DevicePlatform::Android => "android",
                    DevicePlatform::Ios => "ios",
                },
                "profile": profile_str,
                "source": source,
                "count": resolved.len(),
                "device": first.map(|d| &d.name),
                "name": first.map(|d| &d.name),
                "os_version": first.map(|d| &d.os_version),
                "devices": resolved,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

fn resolve_matrix_for_cli(
    config_path: Option<&Path>,
    device_matrix_path: Option<&Path>,
) -> Result<(PathBuf, Option<Vec<String>>)> {
    let mut discovered_matrix = None;
    let mut discovered_tags = None;

    if let Some(config_path) = config_path {
        let cfg = load_config(config_path)?;
        discovered_tags = cfg.device_tags.clone();
        discovered_matrix = Some(cfg.device_matrix);
    } else if device_matrix_path.is_none() {
        let default_config = PathBuf::from("bench-config.toml");
        if default_config.exists()
            && let Ok(cfg) = load_config(&default_config)
        {
            discovered_tags = cfg.device_tags.clone();
            discovered_matrix = Some(cfg.device_matrix);
        }
    }

    let matrix_path = device_matrix_path
        .map(PathBuf::from)
        .or(discovered_matrix)
        .or_else(|| {
            let fallback = PathBuf::from("device-matrix.yaml");
            if fallback.exists() {
                Some(fallback)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            anyhow!("config_error: provide --device-matrix, or provide --config with device_matrix")
        })?;

    Ok((matrix_path, discovered_tags))
}

fn resolve_devices_from_matrix(
    devices: Vec<DeviceEntry>,
    platform: DevicePlatform,
    tags: &[String],
) -> Result<Vec<ResolvedMatrixDevice>> {
    let wanted: Vec<String> = tags
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    let platform_name = match platform {
        DevicePlatform::Android => "android",
        DevicePlatform::Ios => "ios",
    };

    let mut available_tags = BTreeSet::new();
    let mut resolved = Vec::new();

    for device in devices {
        if device.os.trim().to_lowercase() != platform_name {
            continue;
        }
        let normalized_tags: Vec<String> = device
            .tags
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|tag| tag.trim().to_lowercase())
            .filter(|tag| !tag.is_empty())
            .collect();
        for tag in &normalized_tags {
            available_tags.insert(tag.clone());
        }
        let tag_match = wanted.is_empty()
            || normalized_tags
                .iter()
                .any(|tag| wanted.iter().any(|wanted_tag| wanted_tag == tag));
        if !tag_match {
            continue;
        }
        let (identifier, os_version) =
            browserstack_identifier_and_os_version(&device.name, &device.os_version);
        resolved.push(ResolvedMatrixDevice {
            name: device.name,
            os: device.os,
            os_version,
            identifier,
            tags: normalized_tags,
        });
    }

    resolved.sort_by(|a, b| {
        a.identifier
            .cmp(&b.identifier)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.os_version.cmp(&b.os_version))
    });

    if resolved.is_empty() {
        if available_tags.is_empty() {
            bail!(
                "config_error: no devices matched platform `{}` and tags [{}]; no tag metadata found in matrix",
                platform_name,
                wanted.join(", ")
            );
        }
        bail!(
            "config_error: no devices matched platform `{}` and tags [{}]. Available tags: {}",
            platform_name,
            wanted.join(", "),
            available_tags.into_iter().collect::<Vec<_>>().join(", ")
        );
    }

    Ok(resolved)
}

fn browserstack_identifier_and_os_version(name: &str, os_version: &str) -> (String, String) {
    let trimmed_version = os_version.trim();
    if !trimmed_version.is_empty() {
        if let Some(name_version) = parse_ios_version_from_device_identifier(name) {
            let parsed_name = mobench_sdk::codegen::IosDeploymentTarget::parse(name_version);
            let parsed_field = mobench_sdk::codegen::IosDeploymentTarget::parse(trimmed_version);
            if let (Ok(parsed_name), Ok(parsed_field)) = (parsed_name, parsed_field) {
                if parsed_name == parsed_field {
                    return (name.to_string(), trimmed_version.to_string());
                }
            }
        }
        return (
            format!("{}-{}", name, trimmed_version),
            trimmed_version.to_string(),
        );
    }

    if let Some(parsed) = parse_ios_version_from_device_identifier(name) {
        return (name.to_string(), parsed.to_string());
    }

    (name.to_string(), String::new())
}

fn cmd_fixture_init(config_path: &Path, device_matrix_path: &Path, force: bool) -> Result<()> {
    write_config_template(config_path, MobileTarget::Android, force)?;
    write_device_matrix_template(device_matrix_path, force)?;
    println!(
        "Initialized fixture files:\n  - {}\n  - {}",
        config_path.display(),
        device_matrix_path.display()
    );
    Ok(())
}

fn cmd_fixture_build(
    target: SdkTarget,
    release: bool,
    output_dir: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    progress: bool,
) -> Result<()> {
    match target {
        SdkTarget::Android => cmd_build(
            SdkTarget::Android,
            release,
            None,
            None,
            None,
            None,
            output_dir,
            crate_path,
            false,
            false,
            progress,
        )?,
        SdkTarget::Ios => {
            cmd_build(
                SdkTarget::Ios,
                release,
                None,
                None,
                None,
                None,
                output_dir.clone(),
                crate_path,
                false,
                false,
                progress,
            )?;
            cmd_package_ipa(
                "BenchRunner",
                IosSigningMethodArg::Adhoc,
                None,
                None,
                output_dir.clone(),
            )?;
            cmd_package_xcuitest("BenchRunner", None, None, output_dir)?;
        }
        SdkTarget::Both => {
            cmd_build(
                SdkTarget::Android,
                release,
                None,
                None,
                None,
                None,
                output_dir.clone(),
                crate_path.clone(),
                false,
                false,
                progress,
            )?;
            cmd_build(
                SdkTarget::Ios,
                release,
                None,
                None,
                None,
                None,
                output_dir.clone(),
                crate_path,
                false,
                false,
                progress,
            )?;
            cmd_package_ipa(
                "BenchRunner",
                IosSigningMethodArg::Adhoc,
                None,
                None,
                output_dir.clone(),
            )?;
            cmd_package_xcuitest("BenchRunner", None, None, output_dir)?;
        }
    }
    Ok(())
}

fn cmd_fixture_verify_plots(fixture: PlotFixture, output_dir: Option<&Path>) -> Result<()> {
    let (summary_path, default_output_dir, expected_plots): (&str, &str, &[&str]) = match fixture {
        PlotFixture::Basic => (
            "examples/fixtures/basic/summary.json",
            "target/mobench/plot-fixtures/basic",
            &["fibonacci.svg", "checksum.svg"],
        ),
        PlotFixture::Ffi => (
            "examples/fixtures/ffi/summary.json",
            "target/mobench/plot-fixtures/ffi",
            &["fibonacci.svg", "checksum.svg"],
        ),
    };

    let repo = repo_root()?;
    let summary_path = repo.join(summary_path);
    let output_dir = output_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join(default_output_dir));
    let markdown_path = output_dir.join("summary.md");

    if output_dir.exists() {
        fs::remove_dir_all(&output_dir)
            .with_context(|| format!("removing plot fixture output {}", output_dir.display()))?;
    }
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("creating plot fixture output {}", output_dir.display()))?;

    let markdown = cmd_report_summarize(
        &summary_path,
        Some(&markdown_path),
        plots::PlotMode::Require,
    )?;

    if !markdown.contains("## Device Comparison Plots") {
        bail!(
            "expected Device Comparison Plots section in {}",
            markdown_path.display()
        );
    }

    for plot in expected_plots {
        let plot_path = output_dir.join("plots").join(plot);
        if !plot_path.is_file() || fs::metadata(&plot_path)?.len() == 0 {
            bail!("expected rendered plot at {}", plot_path.display());
        }

        let expected_link = format!("](plots/{plot})");
        if !markdown.contains(&expected_link) {
            bail!(
                "expected markdown link {} in {}",
                expected_link,
                markdown_path.display()
            );
        }
    }

    println!(
        "Verified plot fixture {:?} in {}",
        fixture,
        output_dir.display()
    );
    Ok(())
}

fn cmd_fixture_verify(
    config_path: &Path,
    device_matrix_override: Option<&Path>,
    target: SdkTarget,
    profile: Option<String>,
    format: CheckOutputFormat,
) -> Result<()> {
    let mut checks = Vec::new();
    let mut cfg: Option<BenchConfig> = None;
    match load_config(config_path) {
        Ok(parsed) => {
            checks.push(PrereqCheck {
                name: "Run config".to_string(),
                passed: true,
                detail: Some(config_path.display().to_string()),
                fix_hint: None,
            });
            cfg = Some(parsed);
        }
        Err(err) => {
            checks.push(PrereqCheck {
                name: "Run config".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some(format!("Fix config at {}", config_path.display())),
            });
        }
    }

    let matrix_path = device_matrix_override
        .map(PathBuf::from)
        .or_else(|| cfg.as_ref().map(|c| c.device_matrix.clone()));
    if let Some(matrix_path) = matrix_path.as_deref() {
        match load_device_matrix(matrix_path) {
            Ok(matrix) => {
                let mut tags = profile
                    .as_ref()
                    .map(|tag| vec![tag.clone()])
                    .or_else(|| cfg.as_ref().and_then(|c| c.device_tags.clone()))
                    .unwrap_or_else(|| vec!["default".to_string()]);
                tags.retain(|tag| !tag.trim().is_empty());

                let platforms = match target {
                    SdkTarget::Android => vec![DevicePlatform::Android],
                    SdkTarget::Ios => vec![DevicePlatform::Ios],
                    SdkTarget::Both => vec![DevicePlatform::Android, DevicePlatform::Ios],
                };

                let mut unresolved = Vec::new();
                for platform in platforms {
                    if let Err(err) =
                        resolve_devices_from_matrix(matrix.devices.clone(), platform, &tags)
                    {
                        unresolved.push(err.to_string());
                    }
                }
                if unresolved.is_empty() {
                    checks.push(PrereqCheck {
                        name: "Device matrix".to_string(),
                        passed: true,
                        detail: Some(format!(
                            "{} (tags: {})",
                            matrix_path.display(),
                            tags.join(", ")
                        )),
                        fix_hint: None,
                    });
                } else {
                    checks.push(PrereqCheck {
                        name: "Device matrix".to_string(),
                        passed: false,
                        detail: Some(unresolved.join("; ")),
                        fix_hint: Some(format!(
                            "Adjust tags/profile or matrix entries in {}",
                            matrix_path.display()
                        )),
                    });
                }
            }
            Err(err) => checks.push(PrereqCheck {
                name: "Device matrix".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some(format!(
                    "Fix or regenerate device matrix at {}",
                    matrix_path.display()
                )),
            }),
        }
    } else {
        checks.push(PrereqCheck {
            name: "Device matrix".to_string(),
            passed: false,
            detail: Some("missing device matrix path".to_string()),
            fix_hint: Some(
                "Provide --device-matrix or set device_matrix in bench-config.toml".to_string(),
            ),
        });
    }

    let cargo_lock_path = repo_root()?.join("Cargo.lock");
    checks.push(PrereqCheck {
        name: "Cargo.lock".to_string(),
        passed: cargo_lock_path.exists(),
        detail: Some(cargo_lock_path.display().to_string()),
        fix_hint: if cargo_lock_path.exists() {
            None
        } else {
            Some("Run cargo generate-lockfile".to_string())
        },
    });

    let issues = collect_issues(&checks);
    match format {
        CheckOutputFormat::Text => print_check_results_text(&checks, &issues),
        CheckOutputFormat::Json => print_check_results_json(&checks, &issues)?,
    }
    if issues.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} issue(s) found. Fix them and rerun `cargo mobench fixture verify`.",
            issues.len()
        )
    }
}

fn cmd_fixture_cache_key(
    config_path: &Path,
    device_matrix_override: Option<&Path>,
    target: SdkTarget,
    profile: Option<String>,
    format: CheckOutputFormat,
) -> Result<()> {
    let cfg = load_config(config_path)
        .with_context(|| format!("config_error: failed to load {}", config_path.display()))?;
    let matrix_path = device_matrix_override
        .map(PathBuf::from)
        .unwrap_or_else(|| cfg.device_matrix.clone());
    let matrix_bytes = fs::read(&matrix_path).with_context(|| {
        format!(
            "config_error: failed to read device matrix {}",
            matrix_path.display()
        )
    })?;
    let config_bytes = fs::read(config_path)
        .with_context(|| format!("config_error: failed to read {}", config_path.display()))?;
    let cargo_lock_path = repo_root()?.join("Cargo.lock");
    let cargo_lock_bytes = if cargo_lock_path.exists() {
        fs::read(&cargo_lock_path)?
    } else {
        Vec::new()
    };

    let rustc_version = command_version_line("rustc", &["--version"]).unwrap_or_default();
    let cargo_version = command_version_line("cargo", &["--version"]).unwrap_or_default();
    let selected_profile = profile
        .or_else(|| {
            cfg.device_tags
                .clone()
                .and_then(|mut tags| tags.drain(..1).next())
        })
        .unwrap_or_else(|| "default".to_string());

    let mut hasher = Sha256::new();
    hasher.update(format!("mobench={}\n", env!("CARGO_PKG_VERSION")).as_bytes());
    hasher.update(format!("target={target:?}\n").as_bytes());
    hasher.update(format!("profile={selected_profile}\n").as_bytes());
    hasher.update(format!("rustc={rustc_version}\n").as_bytes());
    hasher.update(format!("cargo={cargo_version}\n").as_bytes());
    hasher.update(config_bytes);
    hasher.update(matrix_bytes);
    hasher.update(cargo_lock_bytes);
    let digest = hasher.finalize();
    let cache_key = format!("mobench-fixture-{:x}", digest);

    match format {
        CheckOutputFormat::Text => println!("{cache_key}"),
        CheckOutputFormat::Json => {
            let payload = json!({
                "cache_key": cache_key,
                "target": format!("{target:?}").to_lowercase(),
                "profile": selected_profile,
                "config": config_path.display().to_string(),
                "device_matrix": matrix_path.display().to_string(),
                "rustc": rustc_version,
                "cargo": cargo_version,
                "mobench_version": env!("CARGO_PKG_VERSION"),
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
    }
    Ok(())
}

fn command_version_line(cmd: &str, args: &[&str]) -> Option<String> {
    let mut command = ToolCommand::explicit(cmd).ok()?;
    command.args(args).timeout(Duration::from_secs(30));
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .next()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
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
        assert_eq!(
            layout.default_function.as_deref(),
            Some("zk_mobile_bench::bench_query_proof_generation")
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
