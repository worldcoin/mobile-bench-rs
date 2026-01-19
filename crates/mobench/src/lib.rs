//! # mobench
//!
//! [![Crates.io](https://img.shields.io/crates/v/mobench.svg)](https://crates.io/crates/mobench)
//! [![Documentation](https://docs.rs/mobench/badge.svg)](https://docs.rs/mobench)
//! [![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/worldcoin/mobile-bench-rs/blob/main/LICENSE)
//!
//! Command-line tool for building and running Rust benchmarks on mobile devices.
//!
//! ## Overview
//!
//! `mobench` is the CLI orchestrator for the mobench ecosystem. It handles:
//!
//! - **Building** - Compiles Rust code for Android/iOS and packages mobile apps
//! - **Running** - Executes benchmarks locally or on BrowserStack devices
//! - **Reporting** - Collects and formats benchmark results
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
//! # Run on BrowserStack
//! cargo mobench run --target android --function my_benchmark \
//!     --iterations 100 --warmup 10 --devices "Google Pixel 7-13.0"
//! ```
//!
//! ## Commands
//!
//! | Command | Description |
//! |---------|-------------|
//! | `init` | Initialize a new benchmark project |
//! | `build` | Build mobile artifacts (APK/xcframework) |
//! | `run` | Execute benchmarks locally or on devices |
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
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use browserstack::{BrowserStackAuth, BrowserStackClient};

mod browserstack;
pub mod config;

/// CLI orchestrator for building, packaging, and executing Rust benchmarks on mobile.
#[derive(Parser, Debug)]
#[command(name = "mobench", author, version, about = "Mobile Rust benchmarking orchestrator", long_about = None)]
struct Cli {
    /// Print what would be done without actually doing it
    #[arg(long, global = true)]
    dry_run: bool,

    /// Print verbose output including all commands
    #[arg(long, short = 'v', global = true)]
    verbose: bool,

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
        #[arg(long, help = "Write CSV summary alongside JSON")]
        summary_csv: bool,
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
        #[arg(long, default_value_t = 5)]
        fetch_poll_interval_secs: u64,
        #[arg(long, default_value_t = 300)]
        fetch_timeout_secs: u64,
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
    /// Compare two run summaries for regressions.
    Compare {
        #[arg(long, help = "Baseline JSON summary to compare against")]
        baseline: PathBuf,
        #[arg(long, help = "Candidate JSON summary to compare")]
        candidate: PathBuf,
        #[arg(long, help = "Optional output path for markdown report")]
        output: Option<PathBuf>,
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
        #[arg(long, help = "Output directory for mobile artifacts (default: target/mobench)")]
        output_dir: Option<PathBuf>,
        #[arg(long, help = "Path to the benchmark crate (default: auto-detect bench-mobile/ or crates/{crate})")]
        crate_path: Option<PathBuf>,
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
    summary: SummaryReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    benchmark_results: Option<BTreeMap<String, Vec<Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    performance_metrics: Option<BTreeMap<String, browserstack::PerformanceMetrics>>,
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

pub fn run() -> Result<()> {
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
            summary_csv,
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
                local_only,
            )?;
            let summary_paths = resolve_summary_paths(output.as_deref())?;
            println!(
                "Preparing benchmark run for {:?}: {} (iterations={}, warmup={})",
                spec.target, spec.function, spec.iterations, spec.warmup
            );
            persist_mobile_spec(&spec)?;
            if !spec.devices.is_empty() {
                println!("Devices: {}", spec.devices.join(", "));
            }
            println!("JSON summary will be written to {:?}", summary_paths.json);
            println!(
                "Markdown summary will be written to {:?}",
                summary_paths.markdown
            );
            if summary_csv {
                println!("CSV summary will be written to {:?}", summary_paths.csv);
            }

            // Skip local smoke test - sample-fns uses direct dispatch, not inventory registry
            // Benchmarks will run on the actual mobile device
            println!("Skipping local smoke test - benchmarks will run on mobile device");
            let local_report = json!({
                "skipped": true,
                "reason": "Local smoke test disabled - benchmarks run on mobile device only"
            });
            let mut remote_run = None;
            let artifacts = if local_only {
                println!("Skipping mobile build: --local-only set");
                None
            } else {
                match spec.target {
                    MobileTarget::Android => {
                        let ndk = std::env::var("ANDROID_NDK_HOME").context(
                            "ANDROID_NDK_HOME must be set for Android builds. Example: export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/<version>",
                        )?;
                        let build = run_android_build(&ndk)?;
                        let apk = build.app_path;
                        println!("Built Android APK at {:?}", apk);
                        if spec.devices.is_empty() {
                            println!("Skipping BrowserStack upload/run: no devices provided");
                            Some(MobileArtifacts::Android { apk })
                        } else {
                            let test_apk = build.test_suite_path.as_ref().context(
                                "Android test suite APK missing. Run `cargo mobench build --target android` or `./gradlew assembleDebugAndroidTest` in target/mobench/android",
                            )?;
                            let run = trigger_browserstack_espresso(&spec, &apk, test_apk)?;
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

            run_summary.summary = build_summary(&run_summary)?;
            write_summary(&run_summary, &summary_paths, summary_csv)?;
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
            output_dir,
            crate_path,
        } => {
            cmd_build(target, release, output_dir, crate_path, cli.dry_run, cli.verbose)?;
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
        device_tags: Some(vec!["default".into()]),
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

    let mut session_ids = extract_session_ids(&build_json);
    if session_ids.is_empty() {
        match client.get_json(&sessions_path) {
            Ok(value) => {
                write_json(output_root.join("sessions.json"), &value)?;
                session_ids = extract_session_ids(&value);
            }
            Err(err) => {
                let msg = shorten_html_error(&err.to_string());
                println!("Sessions endpoint unavailable; falling back to build.json: {msg}");
            }
        }
    }

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

#[allow(clippy::too_many_arguments)]
fn resolve_run_spec(
    target: MobileTarget,
    function: String,
    iterations: u32,
    warmup: u32,
    devices: Vec<String>,
    config: Option<&Path>,
    ios_app: Option<PathBuf>,
    ios_test_suite: Option<PathBuf>,
    local_only: bool,
) -> Result<RunSpec> {
    if let Some(cfg_path) = config {
        let cfg = load_config(cfg_path)?;
        let matrix = load_device_matrix(&cfg.device_matrix)?;
        let device_names = match &cfg.device_tags {
            Some(tags) if !tags.is_empty() => filter_devices_by_tags(matrix.devices, tags)?,
            _ => matrix.devices.into_iter().map(|d| d.name).collect(),
        };
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
        bail!("function must not be empty; pass --function <crate::fn> or set function in the config file");
    }

    let ios_xcuitest = match (ios_app, ios_test_suite) {
        (Some(app), Some(test_suite)) => Some(IosXcuitestArtifacts { app, test_suite }),
        (None, None) => None,
        _ => bail!("both --ios-app and --ios-test-suite must be provided together; omit both to let mobench package iOS artifacts when running against devices"),
    };

    let ios_xcuitest = if target == MobileTarget::Ios
        && !local_only
        && !devices.is_empty()
        && ios_xcuitest.is_none()
    {
        Some(package_ios_xcuitest_artifacts()?)
    } else {
        ios_xcuitest
    };

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

fn package_ios_xcuitest_artifacts() -> Result<IosXcuitestArtifacts> {
    let root = repo_root()?;
    let crate_name =
        detect_bench_mobile_crate_name(&root).unwrap_or_else(|_| "bench-mobile".to_string());
    let builder = mobench_sdk::builders::IosBuilder::new(&root, crate_name).verbose(true);
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Ios,
        profile: mobench_sdk::BuildProfile::Debug,
        incremental: true,
    };
    builder
        .build(&cfg)
        .context("Failed to build iOS xcframework before packaging")?;
    let app = builder
        .package_ipa("BenchRunner", mobench_sdk::builders::SigningMethod::AdHoc)
        .context("Failed to package iOS IPA for BrowserStack")?;
    let test_suite = builder
        .package_xcuitest("BenchRunner")
        .context("Failed to package iOS XCUITest runner for BrowserStack")?;
    Ok(IosXcuitestArtifacts { app, test_suite })
}

#[derive(Debug, Clone)]
struct ResolvedBrowserStack {
    username: String,
    access_key: String,
    project: Option<String>,
}

fn trigger_browserstack_espresso(spec: &RunSpec, apk: &Path, test_apk: &Path) -> Result<RemoteRun> {
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

#[derive(Debug)]
struct SummaryPaths {
    json: PathBuf,
    markdown: PathBuf,
    csv: PathBuf,
}

fn resolve_summary_paths(output: Option<&Path>) -> Result<SummaryPaths> {
    let json = output
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| PathBuf::from("run-summary.json"));
    let markdown = json.with_extension("md");
    let csv = json.with_extension("csv");
    Ok(SummaryPaths {
        json,
        markdown,
        csv,
    })
}

fn empty_summary(spec: &RunSpec) -> SummaryReport {
    SummaryReport {
        generated_at: "pending".to_string(),
        generated_at_unix: 0,
        target: spec.target,
        function: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
        devices: spec.devices.clone(),
        device_summaries: Vec::new(),
    }
}

fn build_summary(run_summary: &RunSummary) -> Result<SummaryReport> {
    let generated_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("generating timestamp")?
        .as_secs();
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| generated_at_unix.to_string());

    let mut device_summaries = Vec::new();

    if let Some(results) = &run_summary.benchmark_results {
        for (device, entries) in results {
            let mut benchmarks = Vec::new();
            for entry in entries {
                let function = entry
                    .get("function")
                    .and_then(|f| f.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let samples = extract_samples(entry);
                let stats = compute_sample_stats(&samples);
                let mean_ns = stats
                    .as_ref()
                    .map(|s| s.mean_ns)
                    .or_else(|| entry.get("mean_ns").and_then(|m| m.as_u64()));

                benchmarks.push(BenchmarkStats {
                    function,
                    samples: samples.len(),
                    mean_ns,
                    median_ns: stats.as_ref().map(|s| s.median_ns),
                    p95_ns: stats.as_ref().map(|s| s.p95_ns),
                    min_ns: stats.as_ref().map(|s| s.min_ns),
                    max_ns: stats.as_ref().map(|s| s.max_ns),
                });
            }

            benchmarks.sort_by(|a, b| a.function.cmp(&b.function));
            device_summaries.push(DeviceSummary {
                device: device.clone(),
                benchmarks,
            });
        }
    }

    if device_summaries.is_empty()
        && let Some(local_summary) = summarize_local_report(run_summary)
    {
        device_summaries.push(local_summary);
    }

    Ok(SummaryReport {
        generated_at,
        generated_at_unix,
        target: run_summary.spec.target,
        function: run_summary.spec.function.clone(),
        iterations: run_summary.spec.iterations,
        warmup: run_summary.spec.warmup,
        devices: run_summary.spec.devices.clone(),
        device_summaries,
    })
}

fn write_summary(summary: &RunSummary, paths: &SummaryPaths, summary_csv: bool) -> Result<()> {
    let json = serde_json::to_string_pretty(summary)?;
    ensure_parent_dir(&paths.json)?;
    write_file(&paths.json, json.as_bytes())?;
    println!("Wrote run summary to {:?}", paths.json);

    let markdown = render_markdown_summary(&summary.summary);
    ensure_parent_dir(&paths.markdown)?;
    write_file(&paths.markdown, markdown.as_bytes())?;
    println!("Wrote markdown summary to {:?}", paths.markdown);

    if summary_csv {
        let csv = render_csv_summary(&summary.summary);
        ensure_parent_dir(&paths.csv)?;
        write_file(&paths.csv, csv.as_bytes())?;
        println!("Wrote CSV summary to {:?}", paths.csv);
    }
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("creating directory {:?}", parent))?;
    }
    Ok(())
}

#[derive(Debug)]
struct CompareReport {
    baseline: PathBuf,
    candidate: PathBuf,
    rows: Vec<CompareRow>,
}

#[derive(Debug)]
struct CompareRow {
    device: String,
    function: String,
    baseline_median_ns: Option<u64>,
    candidate_median_ns: Option<u64>,
    median_delta_pct: Option<f64>,
    baseline_p95_ns: Option<u64>,
    candidate_p95_ns: Option<u64>,
    p95_delta_pct: Option<f64>,
}

fn compare_summaries(baseline: &Path, candidate: &Path) -> Result<CompareReport> {
    let baseline_summary = load_run_summary(baseline)?;
    let candidate_summary = load_run_summary(candidate)?;

    let baseline_map = summary_lookup(&baseline_summary.summary);
    let candidate_map = summary_lookup(&candidate_summary.summary);

    let mut rows = Vec::new();
    let mut devices: BTreeMap<String, ()> = BTreeMap::new();
    devices.extend(baseline_map.keys().map(|k| (k.clone(), ())));
    devices.extend(candidate_map.keys().map(|k| (k.clone(), ())));

    for device in devices.keys() {
        let mut functions: BTreeMap<String, ()> = BTreeMap::new();
        if let Some(entry) = baseline_map.get(device) {
            functions.extend(entry.keys().map(|k| (k.clone(), ())));
        }
        if let Some(entry) = candidate_map.get(device) {
            functions.extend(entry.keys().map(|k| (k.clone(), ())));
        }

        for function in functions.keys() {
            let baseline_stats = baseline_map
                .get(device)
                .and_then(|entry| entry.get(function));
            let candidate_stats = candidate_map
                .get(device)
                .and_then(|entry| entry.get(function));

            let baseline_median = baseline_stats.and_then(|s| s.median_ns);
            let candidate_median = candidate_stats.and_then(|s| s.median_ns);
            let median_delta = percent_delta(baseline_median, candidate_median);

            let baseline_p95 = baseline_stats.and_then(|s| s.p95_ns);
            let candidate_p95 = candidate_stats.and_then(|s| s.p95_ns);
            let p95_delta = percent_delta(baseline_p95, candidate_p95);

            rows.push(CompareRow {
                device: device.clone(),
                function: function.clone(),
                baseline_median_ns: baseline_median,
                candidate_median_ns: candidate_median,
                median_delta_pct: median_delta,
                baseline_p95_ns: baseline_p95,
                candidate_p95_ns: candidate_p95,
                p95_delta_pct: p95_delta,
            });
        }
    }

    Ok(CompareReport {
        baseline: baseline.to_path_buf(),
        candidate: candidate.to_path_buf(),
        rows,
    })
}

fn load_run_summary(path: &Path) -> Result<RunSummary> {
    let contents = fs::read_to_string(path).with_context(|| format!("reading {:?}", path))?;
    serde_json::from_str(&contents).with_context(|| format!("parsing summary {:?}", path))
}

fn summary_lookup(summary: &SummaryReport) -> BTreeMap<String, BTreeMap<String, BenchmarkStats>> {
    let mut map = BTreeMap::new();
    for device in &summary.device_summaries {
        let mut functions = BTreeMap::new();
        for bench in &device.benchmarks {
            functions.insert(bench.function.clone(), bench.clone());
        }
        map.insert(device.device.clone(), functions);
    }
    map
}

fn percent_delta(baseline: Option<u64>, candidate: Option<u64>) -> Option<f64> {
    let baseline = baseline? as f64;
    let candidate = candidate? as f64;
    if baseline == 0.0 {
        return None;
    }
    Some(((candidate - baseline) / baseline) * 100.0)
}

fn write_compare_report(report: &CompareReport, output: Option<&Path>) -> Result<()> {
    let markdown = render_compare_markdown(report);
    if let Some(path) = output {
        ensure_parent_dir(path)?;
        write_file(path, markdown.as_bytes())?;
        println!("Wrote compare report to {:?}", path);
    } else {
        println!("{markdown}");
    }
    Ok(())
}

fn render_compare_markdown(report: &CompareReport) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "# Benchmark Comparison");
    let _ = writeln!(output);
    let _ = writeln!(output, "- Baseline: {}", report.baseline.display());
    let _ = writeln!(output, "- Candidate: {}", report.candidate.display());
    let _ = writeln!(output);
    let _ = writeln!(
        output,
        "| Device | Function | Median (base ms) | Median (cand ms) | Median Δ% | P95 (base ms) | P95 (cand ms) | P95 Δ% |"
    );
    let _ = writeln!(
        output,
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |"
    );
    for row in &report.rows {
        let _ = writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            row.device,
            row.function,
            format_ms(row.baseline_median_ns),
            format_ms(row.candidate_median_ns),
            format_delta(row.median_delta_pct),
            format_ms(row.baseline_p95_ns),
            format_ms(row.candidate_p95_ns),
            format_delta(row.p95_delta_pct)
        );
    }
    output
}

fn format_delta(value: Option<f64>) -> String {
    value
        .map(|delta| format!("{:+.2}%", delta))
        .unwrap_or_else(|| "-".to_string())
}

fn summarize_local_report(run_summary: &RunSummary) -> Option<DeviceSummary> {
    let samples = extract_samples(&run_summary.local_report);
    if samples.is_empty() {
        return None;
    }
    let stats = compute_sample_stats(&samples)?;
    let function = run_summary
        .local_report
        .get("spec")
        .and_then(|spec| spec.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or(&run_summary.spec.function)
        .to_string();

    Some(DeviceSummary {
        device: "local".to_string(),
        benchmarks: vec![BenchmarkStats {
            function,
            samples: samples.len(),
            mean_ns: Some(stats.mean_ns),
            median_ns: Some(stats.median_ns),
            p95_ns: Some(stats.p95_ns),
            min_ns: Some(stats.min_ns),
            max_ns: Some(stats.max_ns),
        }],
    })
}

#[derive(Clone, Debug)]
struct SampleStats {
    mean_ns: u64,
    median_ns: u64,
    p95_ns: u64,
    min_ns: u64,
    max_ns: u64,
}

fn compute_sample_stats(samples: &[u64]) -> Option<SampleStats> {
    if samples.is_empty() {
        return None;
    }

    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let len = sorted.len();

    let mean_ns = (sorted.iter().map(|v| *v as u128).sum::<u128>() / len as u128) as u64;
    let median_ns = if len % 2 == 1 {
        sorted[len / 2]
    } else {
        let lower = sorted[(len / 2) - 1];
        let upper = sorted[len / 2];
        (lower + upper) / 2
    };
    let p95_index = percentile_index(len, 0.95);
    let p95_ns = sorted[p95_index];
    let min_ns = sorted[0];
    let max_ns = sorted[len - 1];

    Some(SampleStats {
        mean_ns,
        median_ns,
        p95_ns,
        min_ns,
        max_ns,
    })
}

fn percentile_index(len: usize, percentile: f64) -> usize {
    if len == 0 {
        return 0;
    }
    let rank = (percentile * len as f64).ceil() as usize;
    let index = rank.saturating_sub(1);
    index.min(len - 1)
}

fn extract_samples(value: &Value) -> Vec<u64> {
    let Some(samples) = value.get("samples").and_then(|s| s.as_array()) else {
        return Vec::new();
    };
    let mut durations = Vec::with_capacity(samples.len());
    for sample in samples {
        if let Some(duration) = sample
            .get("duration_ns")
            .and_then(|duration| duration.as_u64())
        {
            durations.push(duration);
        } else if let Some(duration) = sample.as_u64() {
            durations.push(duration);
        }
    }
    durations
}

fn render_markdown_summary(summary: &SummaryReport) -> String {
    let mut output = String::new();
    let devices = if summary.devices.is_empty() {
        "none".to_string()
    } else {
        summary.devices.join(", ")
    };

    let _ = writeln!(output, "# Benchmark Summary");
    let _ = writeln!(output);
    let _ = writeln!(output, "- Generated: {}", summary.generated_at);
    let _ = writeln!(output, "- Target: {:?}", summary.target);
    let _ = writeln!(output, "- Function: {}", summary.function);
    let _ = writeln!(
        output,
        "- Iterations/Warmup: {} / {}",
        summary.iterations, summary.warmup
    );
    let _ = writeln!(output, "- Devices: {}", devices);
    let _ = writeln!(output);

    if summary.device_summaries.is_empty() {
        let _ = writeln!(output, "No benchmark samples were collected.");
        return output;
    }

    for device in &summary.device_summaries {
        let _ = writeln!(output, "## Device: {}", device.device);
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "| Function | Samples | Mean (ms) | Median (ms) | P95 (ms) | Min (ms) | Max (ms) |"
        );
        let _ = writeln!(output, "| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
        for bench in &device.benchmarks {
            let _ = writeln!(
                output,
                "| {} | {} | {} | {} | {} | {} | {} |",
                bench.function,
                bench.samples,
                format_ms(bench.mean_ns),
                format_ms(bench.median_ns),
                format_ms(bench.p95_ns),
                format_ms(bench.min_ns),
                format_ms(bench.max_ns)
            );
        }
        let _ = writeln!(output);
    }

    output
}

fn render_csv_summary(summary: &SummaryReport) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "device,function,samples,mean_ns,median_ns,p95_ns,min_ns,max_ns"
    );
    for device in &summary.device_summaries {
        for bench in &device.benchmarks {
            let _ = writeln!(
                output,
                "{},{},{},{},{},{},{},{}",
                device.device,
                bench.function,
                bench.samples,
                bench.mean_ns.map_or(String::from(""), |v| v.to_string()),
                bench.median_ns.map_or(String::from(""), |v| v.to_string()),
                bench.p95_ns.map_or(String::from(""), |v| v.to_string()),
                bench.min_ns.map_or(String::from(""), |v| v.to_string()),
                bench.max_ns.map_or(String::from(""), |v| v.to_string())
            );
        }
    }
    output
}

fn format_ms(value: Option<u64>) -> String {
    value
        .map(|ns| format!("{:.3}", ns as f64 / 1_000_000.0))
        .unwrap_or_else(|| "-".to_string())
}

fn run_android_build(_ndk_home: &str) -> Result<mobench_sdk::BuildResult> {
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
    Ok(result)
}

fn load_dotenv() {
    if let Ok(root) = repo_root() {
        let path = root.join(".env.local");
        let _ = dotenvy::from_path(path);
    }
}

fn repo_root() -> Result<PathBuf> {
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

/// Build mobile artifacts using mobench-sdk (Phase 1 MVP)
fn cmd_build(
    target: SdkTarget,
    release: bool,
    output_dir: Option<PathBuf>,
    crate_path: Option<PathBuf>,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    // Load config file if present (mobench.toml)
    let config_resolver = config::ConfigResolver::new().unwrap_or_default();
    if let Some(config_path) = &config_resolver.config_path {
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

    let project_root = std::env::current_dir().context("Failed to get current directory")?;

    // Use crate name from config if not auto-detected
    let crate_name = detect_bench_mobile_crate_name(&project_root)
        .or_else(|_| {
            config_resolver
                .crate_name()
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("Could not detect crate name"))
        })
        .unwrap_or_else(|_| "bench-mobile".to_string()); // Fallback for legacy layouts

    // CLI flags override config file values
    let effective_output_dir = output_dir.or_else(|| config_resolver.output_dir().map(|p| p.to_path_buf()));

    if let Some(ref dir) = effective_output_dir {
        println!("  Output: {:?}", dir);
    }
    if let Some(ref path) = crate_path {
        println!("  Crate: {:?}", path);
    }

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
            let mut builder =
                mobench_sdk::builders::AndroidBuilder::new(&project_root, crate_name.clone())
                    .verbose(verbose)
                    .dry_run(dry_run);
            if let Some(ref dir) = effective_output_dir {
                builder = builder.output_dir(dir);
            }
            if let Some(ref path) = crate_path {
                builder = builder.crate_dir(path);
            }
            let result = builder.build(&build_config)?;
            if !dry_run {
                println!("\n[checkmark] Android build completed!");
                println!("  APK: {:?}", result.app_path);
            }
        }
        SdkTarget::Ios => {
            let mut builder =
                mobench_sdk::builders::IosBuilder::new(&project_root, crate_name.clone())
                    .verbose(verbose)
                    .dry_run(dry_run);
            if let Some(ref dir) = effective_output_dir {
                builder = builder.output_dir(dir);
            }
            if let Some(ref path) = crate_path {
                builder = builder.crate_dir(path);
            }
            let result = builder.build(&build_config)?;
            if !dry_run {
                println!("\n[checkmark] iOS build completed!");
                println!("  Framework: {:?}", result.app_path);
            }
        }
        SdkTarget::Both => {
            // Build Android
            let mut android_builder =
                mobench_sdk::builders::AndroidBuilder::new(&project_root, crate_name.clone())
                    .verbose(verbose)
                    .dry_run(dry_run);
            if let Some(ref dir) = effective_output_dir {
                android_builder = android_builder.output_dir(dir);
            }
            if let Some(ref path) = crate_path {
                android_builder = android_builder.crate_dir(path);
            }
            let android_result = android_builder.build(&build_config)?;
            if !dry_run {
                println!("\n[checkmark] Android build completed!");
                println!("  APK: {:?}", android_result.app_path);
            }

            // Build iOS
            let mut ios_builder =
                mobench_sdk::builders::IosBuilder::new(&project_root, crate_name)
                    .verbose(verbose)
                    .dry_run(dry_run);
            if let Some(ref dir) = effective_output_dir {
                ios_builder = ios_builder.output_dir(dir);
            }
            if let Some(ref path) = crate_path {
                ios_builder = ios_builder.crate_dir(path);
            }
            let ios_result = ios_builder.build(&build_config)?;
            if !dry_run {
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

fn detect_bench_mobile_crate_name(root: &Path) -> Result<String> {
    // Try bench-mobile/ first (SDK projects)
    let bench_mobile_path = root.join("bench-mobile").join("Cargo.toml");
    if bench_mobile_path.exists() {
        let contents = fs::read_to_string(&bench_mobile_path)
            .with_context(|| format!("reading bench-mobile manifest at {:?}", bench_mobile_path))?;
        let value: toml::Value = toml::from_str(&contents)
            .with_context(|| format!("parsing bench-mobile manifest {:?}", bench_mobile_path))?;
        let name = value
            .get("package")
            .and_then(|pkg| pkg.get("name"))
            .and_then(|n| n.as_str())
            .ok_or_else(|| {
                anyhow!(
                    "bench-mobile package.name missing in {:?}",
                    bench_mobile_path
                )
            })?;
        return Ok(name.to_string());
    }

    // Fallback: Try crates/sample-fns (repository testing)
    let sample_fns_path = root.join("crates").join("sample-fns").join("Cargo.toml");
    if sample_fns_path.exists() {
        let contents = fs::read_to_string(&sample_fns_path)
            .with_context(|| format!("reading sample-fns manifest at {:?}", sample_fns_path))?;
        let value: toml::Value = toml::from_str(&contents)
            .with_context(|| format!("parsing sample-fns manifest {:?}", sample_fns_path))?;
        let name = value
            .get("package")
            .and_then(|pkg| pkg.get("name"))
            .and_then(|n| n.as_str())
            .ok_or_else(|| anyhow!("sample-fns package.name missing in {:?}", sample_fns_path))?;
        return Ok(name.to_string());
    }

    bail!(
        "No benchmark crate found. Expected bench-mobile/Cargo.toml or crates/sample-fns/Cargo.toml under the project root. Run from the project root or set crate_name in mobench.toml."
    )
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

    let project_root = repo_root()?;
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
    println!(
        "  - Test on BrowserStack: cargo mobench run --target ios --ios-app {:?}",
        ipa_path
    );

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
        let spec = resolve_run_spec(
            MobileTarget::Ios,
            "sample_fns::fibonacci".into(),
            1,
            0,
            vec!["iphone".into()],
            None,
            None,
            None,
            false,
        )
        .expect("should auto-package iOS artifacts when missing");
        let ios_artifacts = spec
            .ios_xcuitest
            .expect("iOS artifacts should be populated");
        assert!(ios_artifacts.app.exists(), "iOS app artifact missing");
        assert!(
            ios_artifacts.test_suite.exists(),
            "iOS test suite artifact missing"
        );
    }
}
