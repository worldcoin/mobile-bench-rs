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
//! | `fixture ...` | Fixture lifecycle helpers (`init`, `build`, `verify`, `cache-key`) |
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
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

use artifacts::{ArtifactLifecycle, validate_default_specs};
use benchmark_output::BenchmarkOutput;
use browserstack::{BrowserStackAuth, BrowserStackClient};
#[cfg(feature = "bench-support")]
pub use ci::bench_support;
pub use ci::{DeviceSelection, Report, RunRequest, RunResult, run_request};
pub(crate) use ci::{
    ci_env, cmd_ci_check_run, cmd_ci_init, cmd_ci_run, cmd_ci_summarize,
    fetch_browserstack_artifacts, infer_pr_number_from_github_ref,
};
#[cfg(test)]
pub(crate) use ci::{
    ci_function_slug, find_baseline_benchmark, merge_ci_target_runs, parse_pr_number_from_ref,
    root_summary_from_merged_targets, summary_report_from_value,
};
#[cfg(test)]
pub(crate) use cli::CiTarget;
pub use cli::MobileTarget;
pub(crate) use cli::{
    CheckOutputFormat, CiCommand, Cli, Command, ConfigCommand, ContractErrorCategory,
    DevicePlatform, DevicesCommand, FixtureCommand, IosSigningMethodArg, ProfileCommand,
    ReportCommand, SdkTarget,
};
#[cfg(test)]
pub(crate) use compare::{CompareReport, CompareRow};
pub(crate) use compare::{
    RegressionFinding, compare_summaries, detect_regressions, inject_compare_into_summary,
    paths_point_to_same_file, render_compare_markdown, resolve_baseline_source,
    snapshot_baseline_for_compare, write_compare_report, write_junit_report,
};
pub(crate) use devices::{
    ResolvedMatrixDevice, cmd_devices, cmd_devices_resolve, resolve_devices_for_profile,
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
pub(crate) use fixtures::{
    cmd_fixture_build, cmd_fixture_cache_key, cmd_fixture_init, cmd_fixture_verify,
    command_version_line,
};
#[cfg(any(test, feature = "bench-support"))]
pub(crate) use reports::render_markdown_summary;
#[cfg(test)]
pub(crate) use reports::{
    MEMORY_BASELINE_GAP_NOTE, extract_benchmark_resource_usage, format_cpu_total_duration_ms,
    format_duration_smart, format_ms, render_summary_markdown_from_output,
    render_summary_markdown_from_output_with_plots_using_python,
};
pub(crate) use reports::{
    append_github_step_summary, append_github_step_summary_from_path, build_summary,
    cmd_report_github, cmd_report_summarize, cmd_summary, compute_sample_stats, empty_summary,
    extract_samples, render_csv_summary, render_summary_markdown_from_output_with_plots,
    resolve_summary_paths, write_summary,
};

mod artifacts;
mod benchmark_output;
mod browserstack;
mod ci;
mod cli;
mod compare;
pub mod config;
mod devices;
mod doctor;
mod fixtures;
mod flamegraph_viewer;
mod github;
mod plots;
mod profile;
mod reports;
pub(crate) mod summarize;

pub(crate) const EXIT_REGRESSION: i32 = 2;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BrowserStackConfig {
    app_automate_username: String,
    app_automate_access_key: String,
    project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    ios_completion_timeout_secs: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct IosXcuitestArtifacts {
    pub(crate) app: PathBuf,
    pub(crate) test_suite: PathBuf,
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
    performance_metrics: Option<BTreeMap<String, browserstack::PerformanceMetrics>>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedProjectLayout {
    pub(crate) project_root: PathBuf,
    pub(crate) crate_dir: PathBuf,
    pub(crate) crate_name: String,
    pub(crate) library_name: String,
    pub(crate) android_abis: Option<Vec<String>>,
    pub(crate) ios_completion_timeout_secs: Option<u64>,
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) output_dir: PathBuf,
    pub(crate) default_function: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProjectLayoutOptions<'a> {
    pub(crate) start_dir: Option<&'a Path>,
    pub(crate) project_root: Option<&'a Path>,
    pub(crate) crate_path: Option<&'a Path>,
    pub(crate) config_path: Option<&'a Path>,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataPackage {
    name: String,
    manifest_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataOutput {
    workspace_root: PathBuf,
    packages: Vec<CargoMetadataPackage>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SummaryReport {
    generated_at: String,
    generated_at_unix: u64,
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    devices: Vec<String>,
    device_summaries: Vec<DeviceSummary>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct DeviceSummary {
    device: String,
    benchmarks: Vec<BenchmarkStats>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BenchmarkStats {
    function: String,
    samples: usize,
    mean_ns: Option<u64>,
    median_ns: Option<u64>,
    p95_ns: Option<u64>,
    min_ns: Option<u64>,
    max_ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_usage: Option<BenchmarkResourceUsage>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct BenchmarkResourceUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu_total_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu_median_ms: Option<u64>,
    /// Legacy alias for `peak_memory_growth_kb`.
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_memory_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_memory_growth_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    process_peak_memory_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_pss_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    private_dirty_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_heap_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    java_heap_kb: Option<u64>,
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
                local_only,
                release,
                cli.dry_run,
            )?;
            let summary_paths = resolve_summary_paths(output.as_deref())?;
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
                persist_mobile_spec(&layout, &spec, release)?;
            }

            // Skip local smoke test - sample-fns uses direct dispatch, not inventory registry
            // Benchmarks will run on the actual mobile device
            if !progress {
                println!("Skipping local smoke test - benchmarks will run on mobile device");
            }
            let local_report = json!({
                "skipped": true,
                "reason": "Local smoke test disabled - benchmarks run on mobile device only"
            });
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
                                    release,
                                    spec.ios_completion_timeout_secs,
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
                performance_metrics: None,
            };

            if cli.dry_run {
                println!();
                println!("[dry-run] Run simulation completed. No changes were made.");
                return Ok(());
            }

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

                let platform = match run_summary.spec.target {
                    MobileTarget::Android => "espresso",
                    MobileTarget::Ios => "xcuitest",
                };

                let dashboard_url = format!(
                    "https://app-automate.browserstack.com/dashboard/v2/builds/{}",
                    build_id
                );

                println!("Waiting for build {} to complete...", build_id);
                println!("Dashboard: {}", dashboard_url);

                match client.wait_and_fetch_all_results_with_poll(
                    build_id,
                    platform,
                    Some(fetch_timeout_secs),
                    Some(fetch_poll_interval_secs),
                ) {
                    Ok((bench_results, perf_metrics)) => {
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
                        println!("\nWarning: Failed to fetch results: {}", e);
                        println!("Build may still be accessible at: {}", dashboard_url);
                    }
                }

                // Also save detailed artifacts to separate directory
                let output_root = fetch_output_dir.join(build_id);
                if let Err(e) = fetch_browserstack_artifacts(
                    &client,
                    run_summary.spec.target,
                    build_id,
                    &output_root,
                    false, // Don't wait again, we already did
                    fetch_poll_interval_secs,
                    fetch_timeout_secs,
                ) {
                    println!("Warning: Failed to fetch detailed artifacts: {}", e);
                }
            } else if fetch {
                println!("No BrowserStack run to fetch (devices not provided?)");
            }

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
            info!("writing benchmark summaries");
            write_summary(
                &run_summary,
                &summary_paths,
                summary_csv,
                plots::PlotMode::Off,
            )?;

            let mut compare_report = None;
            let mut regression_findings: Vec<RegressionFinding> = Vec::new();
            if let Some(baseline_path) = baseline_compare_path.as_deref() {
                let report = compare_summaries(baseline_path, &summary_paths.json)?;
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
            if let Some(report) = &compare_report {
                inject_compare_into_summary(
                    &summary_paths.json,
                    report,
                    regression_threshold_pct,
                    baseline.as_deref(),
                )?;
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
                cmd_ci_run(args, cli.dry_run)?;
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
            project_root,
            output_dir,
            crate_path,
            progress,
        } => {
            cmd_build(
                target,
                release,
                ios_completion_timeout_secs,
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

fn canonicalize_from(base: &Path, path: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    joined
        .canonicalize()
        .with_context(|| format!("resolving path {}", joined.display()))
}

fn resolve_existing_path_arg(base: &Path, path: Option<&Path>) -> Result<Option<PathBuf>> {
    path.map(|value| canonicalize_from(base, value)).transpose()
}

fn cargo_metadata_from(start: &Path) -> Option<CargoMetadataOutput> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(start)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn git_root_from(start: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let path = stdout.trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn config_discovery_base(
    start_dir: &Path,
    explicit_project_root: Option<&PathBuf>,
    explicit_crate_path: Option<&PathBuf>,
) -> PathBuf {
    explicit_project_root
        .cloned()
        .or_else(|| explicit_crate_path.cloned())
        .unwrap_or_else(|| start_dir.to_path_buf())
}

fn load_layout_config(
    start_dir: &Path,
    explicit_project_root: Option<&PathBuf>,
    explicit_crate_path: Option<&PathBuf>,
    explicit_config_path: Option<&PathBuf>,
) -> Result<Option<(config::MobenchConfig, PathBuf)>> {
    if let Some(path) = explicit_config_path {
        return Ok(Some((
            config::MobenchConfig::load_from_file(path)?,
            path.to_path_buf(),
        )));
    }

    let discovery_base =
        config_discovery_base(start_dir, explicit_project_root, explicit_crate_path);
    config::MobenchConfig::discover_from(&discovery_base)
}

fn resolve_project_root_for_layout(
    start_dir: &Path,
    explicit_project_root: Option<PathBuf>,
    explicit_crate_path: Option<&PathBuf>,
    config_path: Option<&Path>,
) -> PathBuf {
    if let Some(root) = explicit_project_root {
        return root;
    }
    if let Some(path) = config_path
        && let Some(parent) = path.parent()
    {
        return parent.to_path_buf();
    }
    if let Some(crate_path) = explicit_crate_path
        && let Some(metadata) = cargo_metadata_from(crate_path)
    {
        return metadata.workspace_root;
    }
    if let Some(metadata) = cargo_metadata_from(start_dir) {
        return metadata.workspace_root;
    }
    if let Some(crate_path) = explicit_crate_path
        && let Some(root) = git_root_from(crate_path)
    {
        return root;
    }
    if let Some(root) = git_root_from(start_dir) {
        return root;
    }
    start_dir.to_path_buf()
}

fn read_package_name_from_dir(dir: &Path) -> Option<String> {
    mobench_sdk::builders::common::read_package_name(&dir.join("Cargo.toml"))
}

fn package_dir_from_metadata(metadata: &CargoMetadataOutput, crate_name: &str) -> Option<PathBuf> {
    metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == crate_name)
        .and_then(|pkg| pkg.manifest_path.parent().map(Path::to_path_buf))
}

fn resolve_configured_crate_dir(project_root: &Path, crate_name: &str) -> Result<Option<PathBuf>> {
    if let Some(pkg_name) = read_package_name_from_dir(project_root)
        && pkg_name == crate_name
    {
        return Ok(Some(project_root.to_path_buf()));
    }

    if let Some(metadata) = cargo_metadata_from(project_root)
        && let Some(dir) = package_dir_from_metadata(&metadata, crate_name)
    {
        return Ok(Some(dir));
    }

    let candidates = [
        project_root.join("crates").join(crate_name),
        project_root.join(crate_name),
        project_root.join("bench-mobile"),
    ];

    for candidate in candidates {
        let manifest = candidate.join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        if read_package_name_from_dir(&candidate).as_deref() == Some(crate_name) {
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

fn resolve_legacy_crate_dir(project_root: &Path) -> Result<PathBuf> {
    let candidates = [
        project_root.to_path_buf(),
        project_root.join("bench-mobile"),
        project_root.join("crates/sample-fns"),
    ];

    for candidate in candidates {
        let manifest = candidate.join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        if read_package_name_from_dir(&candidate).is_some() {
            return Ok(candidate);
        }
    }

    bail!(
        "No benchmark crate found. Pass --crate-path, set [project].crate in mobench.toml, or use a legacy bench-mobile layout."
    )
}

pub(crate) fn resolve_project_layout(
    options: ProjectLayoutOptions<'_>,
) -> Result<ResolvedProjectLayout> {
    let start_dir = match options.start_dir {
        Some(path) => canonicalize_from(Path::new("."), path)?,
        None => std::env::current_dir().context("Failed to get current directory")?,
    };
    let explicit_project_root = resolve_existing_path_arg(&start_dir, options.project_root)?;
    let explicit_crate_path = resolve_existing_path_arg(&start_dir, options.crate_path)?;
    let explicit_config_path = resolve_existing_path_arg(&start_dir, options.config_path)?;

    let loaded_config = load_layout_config(
        &start_dir,
        explicit_project_root.as_ref(),
        explicit_crate_path.as_ref(),
        explicit_config_path.as_ref(),
    )?;
    let (config, config_path) = match loaded_config {
        Some((config, path)) => (Some(config), Some(path)),
        None => (None, None),
    };

    let project_root = resolve_project_root_for_layout(
        &start_dir,
        explicit_project_root,
        explicit_crate_path.as_ref(),
        config_path.as_deref(),
    );

    let crate_dir = if let Some(crate_path) = explicit_crate_path {
        crate_path
    } else if let Some(configured_name) = config
        .as_ref()
        .and_then(|cfg| cfg.project.crate_name.as_deref())
    {
        resolve_configured_crate_dir(&project_root, configured_name)?.ok_or_else(|| {
            anyhow!(
                "Configured benchmark crate '{}' was not found under {}",
                configured_name,
                project_root.display()
            )
        })?
    } else {
        resolve_legacy_crate_dir(&project_root)?
    };

    let crate_name = read_package_name_from_dir(&crate_dir).ok_or_else(|| {
        anyhow!(
            "package.name not found in {}",
            crate_dir.join("Cargo.toml").display()
        )
    })?;
    let library_name = config
        .as_ref()
        .and_then(|cfg| cfg.library_name())
        .unwrap_or_else(|| crate_name.replace('-', "_"));
    let android_abis = config.as_ref().and_then(|cfg| cfg.android.abis.clone());
    let ios_completion_timeout_secs = config
        .as_ref()
        .and_then(|cfg| cfg.browserstack.ios_completion_timeout_secs);
    let output_dir = config
        .as_ref()
        .and_then(|cfg| cfg.project.output_dir.clone())
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                project_root.join(path)
            }
        })
        .unwrap_or_else(|| project_root.join("target/mobench"));
    let default_function = config
        .as_ref()
        .and_then(|cfg| cfg.benchmarks.default_function.clone());

    Ok(ResolvedProjectLayout {
        project_root,
        crate_dir,
        crate_name,
        library_name,
        android_abis,
        ios_completion_timeout_secs,
        config_path,
        output_dir,
        default_function,
    })
}

fn discover_benchmarks_for_layout(layout: &ResolvedProjectLayout) -> Result<Vec<String>> {
    let mut benchmarks =
        mobench_sdk::codegen::detect_all_benchmarks(&layout.crate_dir, &layout.crate_name);
    benchmarks.sort();
    benchmarks.dedup();
    Ok(benchmarks)
}

fn ensure_verify_smoke_test_supported(layout: &ResolvedProjectLayout) -> Result<()> {
    let supported_embedded_crates = ["sample-fns", "basic-benchmark", "ffi-benchmark"];
    if supported_embedded_crates.contains(&layout.crate_name.as_str()) {
        return Ok(());
    }

    bail!(
        "verify --smoke-test is unsupported for external crate '{}'; smoke tests only work for benchmark crates linked into the mobench CLI binary",
        layout.crate_name
    )
}

fn configured_android_abis(layout: &ResolvedProjectLayout) -> Vec<String> {
    layout
        .android_abis
        .as_ref()
        .filter(|abis| !abis.is_empty())
        .cloned()
        .unwrap_or_else(|| vec!["arm64-v8a".to_string()])
}

fn configured_ios_completion_timeout_secs(
    layout: &ResolvedProjectLayout,
    ios_completion_timeout_secs: Option<u64>,
) -> Option<u64> {
    ios_completion_timeout_secs.or(layout.ios_completion_timeout_secs)
}

fn write_config_template(path: &Path, target: MobileTarget, overwrite: bool) -> Result<()> {
    ensure_can_write(path, overwrite)?;

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
        device_tags: Some(vec!["default".into()]),
        browserstack: BrowserStackConfig {
            app_automate_username: "${BROWSERSTACK_USERNAME}".into(),
            app_automate_access_key: "${BROWSERSTACK_ACCESS_KEY}".into(),
            project: Some("mobile-bench-rs".into()),
            ios_completion_timeout_secs: None,
        },
        ios_xcuitest,
    };

    let contents = toml::to_string_pretty(&cfg)?;
    write_file(path, contents.as_bytes())
}

fn write_device_matrix_template(path: &Path, overwrite: bool) -> Result<()> {
    ensure_can_write(path, overwrite)?;

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

#[allow(clippy::too_many_arguments)]
fn resolve_run_spec(
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    devices: Vec<String>,
    layout: &ResolvedProjectLayout,
    config: Option<&Path>,
    device_matrix: Option<&Path>,
    device_tags: Vec<String>,
    ios_app: Option<PathBuf>,
    ios_test_suite: Option<PathBuf>,
    ios_completion_timeout_secs: Option<u64>,
    local_only: bool,
    _release: bool,
    dry_run: bool,
) -> Result<RunSpec> {
    if let Some(cfg_path) = config {
        let cfg = load_config(cfg_path)?;
        let configured_ios_completion_timeout_secs = ios_completion_timeout_secs
            .or(cfg.browserstack.ios_completion_timeout_secs)
            .or(layout.ios_completion_timeout_secs);
        let matrix_path = device_matrix.map(Path::to_path_buf).unwrap_or_else(|| {
            resolve_project_relative_path(
                cfg_path.parent().unwrap_or_else(|| Path::new(".")),
                cfg.device_matrix.as_path(),
            )
        });
        let matrix = load_device_matrix(&matrix_path)?;
        let resolved_tags = if !device_tags.is_empty() {
            Some(device_tags)
        } else {
            cfg.device_tags.clone()
        };
        let device_names = match resolved_tags.as_ref() {
            Some(tags) if !tags.is_empty() => filter_devices_by_tags(matrix.devices, tags)?,
            _ => matrix.devices.into_iter().map(|d| d.name).collect(),
        };
        return Ok(RunSpec {
            target: cfg.target,
            function: cfg.function,
            iterations: cfg.iterations,
            warmup: cfg.warmup,
            devices: device_names,
            ios_completion_timeout_secs: configured_ios_completion_timeout_secs,
            browserstack: Some(cfg.browserstack),
            ios_xcuitest: cfg.ios_xcuitest,
        });
    }

    if function.trim().is_empty() {
        bail!(
            "function must not be empty; pass --function <crate::fn> or set function in the config file"
        );
    }

    if device_matrix.is_some() && !devices.is_empty() {
        bail!("--device-matrix cannot be combined with --devices; choose one source for devices");
    }
    if device_matrix.is_none() && !device_tags.is_empty() {
        bail!("--device-tags requires --device-matrix or a config file with device tags");
    }

    let resolved_devices = if !devices.is_empty() {
        devices
    } else if let Some(matrix_path) = device_matrix {
        let matrix = load_device_matrix(matrix_path)?;
        if device_tags.is_empty() {
            matrix.devices.into_iter().map(|d| d.name).collect()
        } else {
            filter_devices_by_tags(matrix.devices, &device_tags)?
        }
    } else {
        Vec::new()
    };

    let ios_xcuitest = match (ios_app, ios_test_suite) {
        (Some(app), Some(test_suite)) => Some(IosXcuitestArtifacts { app, test_suite }),
        (None, None) => None,
        _ => bail!(
            "both --ios-app and --ios-test-suite must be provided together; omit both to let mobench package iOS artifacts when running against devices"
        ),
    };

    let ios_xcuitest = if target == MobileTarget::Ios
        && !local_only
        && !resolved_devices.is_empty()
        && ios_xcuitest.is_none()
    {
        if dry_run {
            println!("📦 [dry-run] Would auto-package iOS artifacts for BrowserStack...");
        }
        Some(default_ios_xcuitest_artifacts(layout))
    } else {
        ios_xcuitest
    };

    Ok(RunSpec {
        target,
        function,
        iterations,
        warmup,
        devices: resolved_devices,
        ios_completion_timeout_secs: configured_ios_completion_timeout_secs(
            layout,
            ios_completion_timeout_secs,
        ),
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

fn filter_devices_by_tags(devices: Vec<DeviceEntry>, tags: &[String]) -> Result<Vec<String>> {
    let wanted: Vec<String> = tags
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    if wanted.is_empty() {
        return Ok(devices.into_iter().map(|d| d.name).collect());
    }

    let mut matched = Vec::new();
    let mut available_tags = BTreeSet::new();
    for device in devices {
        let Some(device_tags) = device.tags.as_ref() else {
            continue;
        };
        for tag in device_tags {
            let normalized = tag.trim().to_lowercase();
            if !normalized.is_empty() {
                available_tags.insert(normalized);
            }
        }
        let has_match = device_tags.iter().any(|tag| {
            let candidate = tag.trim().to_lowercase();
            wanted.iter().any(|wanted_tag| wanted_tag == &candidate)
        });
        if has_match {
            matched.push(device.name);
        }
    }

    if matched.is_empty() {
        if available_tags.is_empty() {
            bail!(
                "no devices matched tags [{}] in device matrix; no tag metadata found in the matrix",
                wanted.join(", ")
            );
        }
        let available = available_tags.into_iter().collect::<Vec<_>>().join(", ");
        bail!(
            "no devices matched tags [{}] in device matrix. Available tags: {}",
            wanted.join(", "),
            available
        );
    }
    Ok(matched)
}

pub(crate) fn with_ios_benchmark_timeout_env<T>(
    timeout_secs: Option<u64>,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let Some(timeout_secs) = timeout_secs else {
        return f();
    };

    let previous = env::var_os("MOBENCH_IOS_BENCHMARK_TIMEOUT_SECS");
    println!("Using iOS benchmark completion timeout: {timeout_secs}s");

    unsafe {
        env::set_var(
            "MOBENCH_IOS_BENCHMARK_TIMEOUT_SECS",
            timeout_secs.to_string(),
        )
    };

    let result = f();

    match previous {
        Some(value) => unsafe { env::set_var("MOBENCH_IOS_BENCHMARK_TIMEOUT_SECS", value) },
        None => unsafe { env::remove_var("MOBENCH_IOS_BENCHMARK_TIMEOUT_SECS") },
    }

    result
}

pub(crate) fn run_ios_build(
    layout: &ResolvedProjectLayout,
    release: bool,
    dry_run: bool,
    ios_completion_timeout_secs: Option<u64>,
) -> Result<(PathBuf, PathBuf)> {
    ArtifactLifecycle::new(layout, None, ios_completion_timeout_secs)
        .run_ios_build(release, dry_run)
}

fn package_ios_xcuitest_artifacts(
    layout: &ResolvedProjectLayout,
    release: bool,
    ios_completion_timeout_secs: Option<u64>,
) -> Result<IosXcuitestArtifacts> {
    ArtifactLifecycle::new(layout, None, ios_completion_timeout_secs)
        .package_ios_xcuitest_artifacts(release)
        .context("Failed to package iOS XCUITest artifacts for BrowserStack")
}

fn default_ios_xcuitest_artifacts(layout: &ResolvedProjectLayout) -> IosXcuitestArtifacts {
    ArtifactLifecycle::new(layout, None, None).default_ios_xcuitest_artifacts()
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
    ArtifactLifecycle::new(layout, None, None).uses_managed_ios_xcuitest_artifacts(artifacts)
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

            let mean_ns = benchmark
                .get("mean_ns")
                .and_then(|m| m.as_u64())
                .unwrap_or(0);

            let samples: Vec<u64> = benchmark
                .get("samples")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.get("duration_ns").and_then(|d| d.as_u64()))
                        .collect()
                })
                .unwrap_or_default();

            let sample_count = samples.len();
            let min_ns = samples.iter().copied().min();
            let max_ns = samples.iter().copied().max();

            let std_dev_ns = if sample_count > 1 {
                let mean = mean_ns as f64;
                let variance: f64 = samples
                    .iter()
                    .map(|&s| {
                        let diff = s as f64 - mean;
                        diff * diff
                    })
                    .sum::<f64>()
                    / (sample_count - 1) as f64;
                Some(variance.sqrt() as u64)
            } else {
                None
            };

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

    // Upload the app-under-test APK.
    let upload = client.upload_espresso_app(apk)?;

    // Upload the Espresso test-suite APK produced by Gradle.
    let test_upload = client.upload_espresso_test_suite(test_apk)?;

    // Schedule the Espresso build with both app and testSuite, as required by BrowserStack.
    let run = client.schedule_espresso_run(
        &spec.devices,
        &upload.app_url,
        &test_upload.test_suite_url,
    )?;

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
        app_url: upload.app_url,
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

    let app_upload = client.upload_xcuitest_app(&artifacts.app)?;
    let test_upload = client.upload_xcuitest_test_suite(&artifacts.test_suite)?;
    let run = client.schedule_xcuitest_run(
        &spec.devices,
        &app_upload.app_url,
        &test_upload.test_suite_url,
    )?;

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
    release: bool,
) -> Result<()> {
    artifacts::persist_mobile_spec(&ArtifactLifecycle::new(layout, None, None), spec, release)
}

pub(crate) fn run_android_build(
    layout: &ResolvedProjectLayout,
    _ndk_home: &str,
    release: bool,
    dry_run: bool,
) -> Result<mobench_sdk::BuildResult> {
    ensure_android_home();
    ArtifactLifecycle::new(layout, None, None).run_android_build(release, dry_run)
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
        let _ = dotenvy::from_path_override(root.join(".env.local"));
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
        let _ = dotenvy::from_path_override(dir.join(".env.local"));
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
    let lifecycle = ArtifactLifecycle::new(&layout, output_dir, ios_completion_timeout_secs);

    // Progress mode: simplified output
    if progress {
        let build_config = lifecycle.build_config(target, release);

        match target {
            SdkTarget::Android => {
                println!("[1/3] Building Rust library...");
                let builder = lifecycle.android_builder(false, dry_run);
                println!("[2/3] Building Android APK...");
                let result = builder.build(&build_config)?;
                println!("[3/3] Done!");
                if !dry_run {
                    println!("\n\u{2713} APK: {:?}", result.app_path);
                }
            }
            SdkTarget::Ios => {
                println!("[1/3] Building Rust library...");
                let builder = lifecycle.ios_builder(false, dry_run);
                println!("[2/3] Building iOS xcframework...");
                let result = with_ios_benchmark_timeout_env(
                    lifecycle.ios_completion_timeout_secs(),
                    || Ok(builder.build(&build_config)?),
                )?;
                println!("[3/3] Done!");
                if !dry_run {
                    println!("\n\u{2713} Framework: {:?}", result.app_path);
                }
            }
            SdkTarget::Both => {
                println!("[1/5] Building Rust library for Android...");
                let android_builder = lifecycle.android_builder(false, dry_run);
                println!("[2/5] Building Android APK...");
                let android_result = android_builder.build(&build_config)?;

                println!("[3/5] Building Rust library for iOS...");
                let ios_builder = lifecycle.ios_builder(false, dry_run);
                println!("[4/5] Building iOS xcframework...");
                let ios_result = with_ios_benchmark_timeout_env(
                    lifecycle.ios_completion_timeout_secs(),
                    || Ok(ios_builder.build(&build_config)?),
                )?;

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

    println!("  Output: {:?}", lifecycle.output_dir());
    println!("  Project root: {:?}", layout.project_root);
    println!("  Crate: {:?}", layout.crate_dir);

    let build_config = lifecycle.build_config(target, release);

    match target {
        SdkTarget::Android => {
            println!("\nBuilding for Android...");
            println!("  Building Rust library for Android targets...");
            let builder = lifecycle.android_builder(verbose, dry_run);
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
            let builder = lifecycle.ios_builder(verbose, dry_run);
            let result =
                with_ios_benchmark_timeout_env(lifecycle.ios_completion_timeout_secs(), || {
                    Ok(builder.build(&build_config)?)
                })?;
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
            let android_builder = lifecycle.android_builder(verbose, dry_run);
            let android_result = android_builder.build(&build_config)?;
            if !dry_run {
                println!("\u{2713} Built Android APK");
                println!("\n[checkmark] Android build completed!");
                println!("  APK: {:?}", android_result.app_path);
            }

            // Build iOS
            println!("\nBuilding for iOS...");
            println!("  Building Rust library for iOS targets...");
            let ios_builder = lifecycle.ios_builder(verbose, dry_run);
            let ios_result =
                with_ios_benchmark_timeout_env(lifecycle.ios_completion_timeout_secs(), || {
                    Ok(ios_builder.build(&build_config)?)
                })?;
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
    let lifecycle = ArtifactLifecycle::new(&layout, output_dir, None);
    let ipa_path = lifecycle
        .package_ipa(scheme, method)
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
    let lifecycle = ArtifactLifecycle::new(&layout, output_dir, None);
    let zip_path = lifecycle
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
    let lifecycle = ArtifactLifecycle::new(&layout, output_dir, None);

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
        let specs = validate_default_specs(&lifecycle);
        for (index, (path, result)) in specs.iter().enumerate() {
            if index == 0 {
                println!("OK (found at default locations)");
            }
            match result {
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
        if !specs.is_empty() {
            checks_passed += 1;
        } else {
            println!("SKIPPED (no spec file found, use --spec-path to specify)");
            warnings += 1;
        }
    }

    // 3. Check artifacts if requested
    print!("  [3/4] Checking build artifacts... ");
    if check_artifacts {
        let (artifacts_ok, artifact_details) = lifecycle.artifact_details(target);

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
                    let mean_ns = if samples > 0 {
                        report.samples.iter().map(|s| s.duration_ns).sum::<u64>() / samples as u64
                    } else {
                        0
                    };
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
                    let mean_ns = if samples > 0 {
                        report.samples.iter().map(|s| s.duration_ns).sum::<u64>() / samples as u64
                    } else {
                        0
                    };
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
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(100);

    let warmup = value
        .get("warmup")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(10);

    // Validate
    if name.trim().is_empty() {
        bail!("spec.name/function is empty");
    }
    if iterations == 0 {
        bail!("spec.iterations must be > 0");
    }

    Ok(mobench_sdk::BenchSpec {
        name,
        iterations,
        warmup,
    })
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

#[cfg(test)]
mod tests;
