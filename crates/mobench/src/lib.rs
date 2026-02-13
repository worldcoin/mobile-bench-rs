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
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fmt::Write;
use std::fs;
use std::io::Write as IoWrite;
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

    /// Assume yes to prompts and allow overwriting files
    #[arg(long, global = true)]
    yes: bool,

    /// Disable interactive prompts (fail instead)
    #[arg(long, global = true)]
    non_interactive: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run benchmarks on real devices via BrowserStack.
    ///
    /// This is a single-command flow that:
    /// 1. Builds Rust libraries for the target platform
    /// 2. Packages mobile apps (APK/IPA) automatically
    /// 3. Uploads to BrowserStack
    /// 4. Schedules the benchmark run
    /// 5. Fetches results when complete
    ///
    /// For iOS, IPA and XCUITest packages are created automatically unless
    /// you provide --ios-app and --ios-test-suite to override.
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
        #[arg(long, help = "Device matrix YAML file to load device names from")]
        device_matrix: Option<PathBuf>,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Device tags to select from the device matrix (comma-separated or repeatable)"
        )]
        device_tags: Vec<String>,
        #[arg(long, help = "Optional path to config file")]
        config: Option<PathBuf>,
        #[arg(long, help = "Optional output path for JSON report")]
        output: Option<PathBuf>,
        #[arg(long, help = "Write CSV summary alongside JSON")]
        summary_csv: bool,
        #[arg(
            long,
            help = "Enable CI mode (job summary, optional JUnit, regression exit codes)"
        )]
        ci: bool,
        #[arg(long, help = "Baseline JSON summary to compare for regressions")]
        baseline: Option<PathBuf>,
        #[arg(
            long,
            default_value_t = 5.0,
            help = "Regression threshold percentage when comparing to baseline"
        )]
        regression_threshold_pct: f64,
        #[arg(long, help = "Write JUnit XML report to the given path")]
        junit: Option<PathBuf>,
        #[arg(long, help = "Skip mobile builds and only run the host harness")]
        local_only: bool,
        #[arg(
            long,
            help = "Build in release mode (recommended for BrowserStack to reduce APK size and upload time)"
        )]
        release: bool,
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
        #[arg(long, help = "Show simplified step-by-step progress output")]
        progress: bool,
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
    /// Validate local + CI prerequisites and configuration.
    Doctor {
        #[arg(long, value_enum, default_value_t = SdkTarget::Both)]
        target: SdkTarget,
        #[arg(long, help = "Optional path to run config file to validate")]
        config: Option<PathBuf>,
        #[arg(long, help = "Optional path to device matrix YAML file to validate")]
        device_matrix: Option<PathBuf>,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Device tags to select from the device matrix (comma-separated or repeatable)"
        )]
        device_tags: Vec<String>,
        #[arg(
            long,
            default_value_t = true,
            action = clap::ArgAction::Set,
            num_args = 0..=1,
            default_missing_value = "true",
            help = "Validate BrowserStack credentials"
        )]
        browserstack: bool,
        #[arg(long, value_enum, default_value_t = CheckOutputFormat::Text)]
        format: CheckOutputFormat,
    },
    /// CI helpers (workflow and action scaffolding).
    Ci {
        #[command(subcommand)]
        command: CiCommand,
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
        #[arg(
            long,
            help = "Output directory for mobile artifacts (default: target/mobench)"
        )]
        output_dir: Option<PathBuf>,
        #[arg(
            long,
            help = "Path to the benchmark crate (default: auto-detect bench-mobile/ or crates/{crate})"
        )]
        crate_path: Option<PathBuf>,
        #[arg(long, help = "Show simplified step-by-step progress output")]
        progress: bool,
    },
    /// Package iOS app as IPA for distribution or testing.
    PackageIpa {
        #[arg(long, default_value = "BenchRunner", help = "Xcode scheme to build")]
        scheme: String,
        #[arg(long, value_enum, default_value = "adhoc", help = "Signing method")]
        method: IosSigningMethodArg,
        #[arg(
            long,
            help = "Output directory for mobile artifacts (default: target/mobench)"
        )]
        output_dir: Option<PathBuf>,
    },
    /// Package XCUITest runner for BrowserStack testing.
    ///
    /// Builds the XCUITest runner using xcodebuild and zips the resulting
    /// .xctest bundle for BrowserStack upload. The output is placed at
    /// `target/mobench/ios/BenchRunnerUITests.zip` by default.
    PackageXcuitest {
        #[arg(long, default_value = "BenchRunner", help = "Xcode scheme to build")]
        scheme: String,
        #[arg(
            long,
            help = "Output directory for mobile artifacts (default: target/mobench)"
        )]
        output_dir: Option<PathBuf>,
    },
    /// List all discovered benchmark functions (Phase 1 MVP).
    List,
    /// Verify benchmark setup: registry, spec, artifacts, and optional smoke test.
    ///
    /// This command validates:
    /// - Registry has benchmark functions registered
    /// - Spec file exists and is valid (if --spec-path provided)
    /// - Artifacts are present and consistent (if --check-artifacts)
    /// - Runs a local smoke test (if --smoke-test and function is specified)
    Verify {
        #[arg(long, value_enum, help = "Target platform to verify artifacts for")]
        target: Option<SdkTarget>,
        #[arg(long, help = "Path to bench_spec.json to validate")]
        spec_path: Option<PathBuf>,
        #[arg(long, help = "Check that build artifacts exist")]
        check_artifacts: bool,
        #[arg(long, help = "Run a local smoke test with minimal iterations")]
        smoke_test: bool,
        #[arg(long, help = "Function name to verify/smoke test")]
        function: Option<String>,
        #[arg(
            long,
            help = "Output directory for mobile artifacts (default: target/mobench)"
        )]
        output_dir: Option<PathBuf>,
    },
    /// Display summary statistics from a benchmark report JSON file.
    ///
    /// Prints avg/min/max/median, sample count, device, and OS version
    /// from the specified report file.
    Summary {
        #[arg(help = "Path to the benchmark report JSON file")]
        report: PathBuf,
        #[arg(long, help = "Output format: text (default), json, or csv")]
        format: Option<SummaryFormat>,
    },
    /// List available BrowserStack devices for testing.
    ///
    /// Fetches and displays the list of available devices from BrowserStack
    /// that can be used with the --devices flag in the run command.
    ///
    /// Examples:
    ///   mobench devices                    # List all devices
    ///   mobench devices --platform android # List Android devices only
    ///   mobench devices --json             # Output as JSON
    ///   mobench devices --validate "Google Pixel 7-13.0"  # Validate a device spec
    Devices {
        #[arg(long, value_enum, help = "Filter by platform (android or ios)")]
        platform: Option<DevicePlatform>,
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(long, help = "Validate device specs against available devices")]
        validate: Vec<String>,
    },
    /// Check prerequisites for building mobile artifacts.
    ///
    /// Validates that all required tools and configurations are in place
    /// before attempting a build. This includes checking for:
    ///
    /// - Android: ANDROID_NDK_HOME, cargo-ndk, Rust targets
    /// - iOS: Xcode, xcodegen, Rust targets
    /// - Both: cargo, rustup
    ///
    /// Examples:
    ///   cargo mobench check --target android
    ///   cargo mobench check --target ios
    ///   cargo mobench check --target android --format json
    Check {
        /// Target platform (android or ios)
        #[arg(long, short, value_enum)]
        target: SdkTarget,
        /// Output format (text or json)
        #[arg(long, default_value = "text")]
        format: CheckOutputFormat,
    },
}

#[derive(Subcommand, Debug)]
enum CiCommand {
    /// Generate GitHub Actions workflow + local action wrapper.
    Init {
        #[arg(
            long,
            default_value = ".github/workflows/mobile-bench.yml",
            help = "Path to write the workflow file"
        )]
        workflow: PathBuf,
        #[arg(
            long,
            default_value = ".github/actions/mobench",
            help = "Directory to write the local GitHub Action"
        )]
        action_dir: PathBuf,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lowercase")]
enum DevicePlatform {
    Android,
    Ios,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lowercase")]
enum SummaryFormat {
    Text,
    Json,
    Csv,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lowercase")]
enum CheckOutputFormat {
    Text,
    Json,
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
            fetch,
            fetch_output_dir,
            fetch_poll_interval_secs,
            fetch_timeout_secs,
            progress,
        } => {
            let spec = resolve_run_spec(
                target,
                function,
                iterations,
                warmup,
                devices,
                config.as_deref(),
                device_matrix.as_deref(),
                device_tags,
                ios_app,
                ios_test_suite,
                local_only,
                release,
            )?;
            let summary_paths = resolve_summary_paths(output.as_deref())?;
            let root = repo_root()?;
            let output_dir = root.join("target/mobench");

            // Validate device specs early to catch errors before building (C2: Device validation)
            if !spec.devices.is_empty() && !local_only {
                if let Ok(creds) = resolve_browserstack_credentials(spec.browserstack.as_ref()) {
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
                validate_benchmark_function(&root, &spec.function)?;
            }

            // Persist the spec and metadata to mobile app bundles
            if progress {
                println!("[1/4] Preparing benchmark spec...");
            }
            persist_mobile_spec(&spec, release)?;

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
                        if progress {
                            println!("[2/4] Building Android APK...");
                        } else {
                            println!("Building for Android...");
                            println!("  Building Rust library for Android targets...");
                        }
                        let ndk = std::env::var("ANDROID_NDK_HOME").context(
                            "ANDROID_NDK_HOME must be set for Android builds. Example: export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/<version>",
                        )?;
                        let build = run_android_build(&ndk, release)?;
                        let apk = build.app_path;
                        if !progress {
                            println!("\u{2713} Built Android APK at {:?}", apk);
                        }
                        if spec.devices.is_empty() {
                            if !progress {
                                println!("Skipping BrowserStack upload/run: no devices provided");
                            }
                            Some(MobileArtifacts::Android { apk })
                        } else {
                            if progress {
                                println!("[3/4] Uploading to BrowserStack...");
                            }
                            let test_apk = build.test_suite_path.as_ref().context(
                                "Android test suite APK missing. Run `cargo mobench build --target android` or `./gradlew assembleDebugAndroidTest` in target/mobench/android",
                            )?;
                            let run = trigger_browserstack_espresso(&spec, &apk, test_apk)?;
                            remote_run = Some(run);
                            Some(MobileArtifacts::Android { apk })
                        }
                    }
                    MobileTarget::Ios => {
                        if progress {
                            println!("[2/4] Building iOS xcframework...");
                        } else {
                            println!("Building for iOS...");
                            println!("  Building Rust library for iOS targets...");
                        }
                        let (xcframework, header) = run_ios_build(release)?;
                        if !progress {
                            println!("\u{2713} Built iOS xcframework at {:?}", xcframework);
                        }
                        let ios_xcuitest = spec.ios_xcuitest.clone();

                        if spec.devices.is_empty() {
                            if !progress {
                                println!("Skipping BrowserStack upload/run: no devices provided");
                            }
                        } else {
                            if progress {
                                println!("[3/4] Uploading to BrowserStack...");
                            }
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

            let mut compare_report = None;
            let mut regression_findings: Vec<RegressionFinding> = Vec::new();
            if let Some(baseline_path) = baseline.as_deref() {
                let report = compare_summaries(baseline_path, &summary_paths.json)?;
                regression_findings = detect_regressions(&report, regression_threshold_pct);
                compare_report = Some(report);
            }

            if ci {
                if let Err(err) = append_github_step_summary_from_path(&summary_paths.markdown) {
                    eprintln!("Warning: failed to publish job summary: {err}");
                }
                if let Some(report) = &compare_report {
                    let compare_markdown = render_compare_markdown(report);
                    if let Ok(summary_path) = env::var("GITHUB_STEP_SUMMARY") {
                        if let Err(err) =
                            append_github_step_summary(&compare_markdown, &summary_path)
                        {
                            eprintln!("Warning: failed to append comparison report: {err}");
                        }
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
            output_dir,
            crate_path,
            progress,
        } => {
            cmd_build(
                target,
                release,
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
            output_dir,
        } => {
            cmd_package_ipa(&scheme, method, output_dir)?;
        }
        Command::PackageXcuitest { scheme, output_dir } => {
            cmd_package_xcuitest(&scheme, output_dir)?;
        }
        Command::List => {
            cmd_list()?;
        }
        Command::Verify {
            target,
            spec_path,
            check_artifacts,
            smoke_test,
            function,
            output_dir,
        } => {
            cmd_verify(
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
            platform,
            json,
            validate,
        } => {
            cmd_devices(platform, json, validate)?;
        }
        Command::Check { target, format } => {
            cmd_check(target, format)?;
        }
    }

    Ok(())
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

const CI_WORKFLOW_TEMPLATE: &str = include_str!("../templates/ci/mobile-bench.yml");
const CI_ACTION_TEMPLATE: &str = include_str!("../templates/ci/action.yml");
const CI_ACTION_README_TEMPLATE: &str = include_str!("../templates/ci/action.README.md");

fn cmd_ci_init(workflow_path: &Path, action_dir: &Path, overwrite: bool) -> Result<()> {
    let action_yaml = action_dir.join("action.yml");
    let action_readme = action_dir.join("README.md");

    ensure_can_write(workflow_path, overwrite)?;
    ensure_can_write(&action_yaml, overwrite)?;
    ensure_can_write(&action_readme, overwrite)?;

    write_file(workflow_path, CI_WORKFLOW_TEMPLATE.as_bytes())?;
    write_file(&action_yaml, CI_ACTION_TEMPLATE.as_bytes())?;
    write_file(&action_readme, CI_ACTION_README_TEMPLATE.as_bytes())?;

    println!("Wrote workflow to {}", workflow_path.display());
    println!("Wrote GitHub Action to {}", action_yaml.display());
    println!("Wrote GitHub Action README to {}", action_readme.display());
    Ok(())
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
    // First, try iOS-style markers: BENCH_REPORT_JSON_START ... BENCH_REPORT_JSON_END
    // This allows multi-line JSON and is more robust for iOS NSLog output
    if let Some(json) = extract_bench_json_ios_markers(contents) {
        return Some(json);
    }

    // Fall back to Android-style single-line marker: BENCH_JSON {...}
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

/// Extract benchmark JSON from iOS logs using START/END markers.
/// iOS uses NSLog which may split the JSON across multiple log lines,
/// so we need to capture everything between the markers.
fn extract_bench_json_ios_markers(contents: &str) -> Option<Value> {
    let start_marker = "BENCH_REPORT_JSON_START";
    let end_marker = "BENCH_REPORT_JSON_END";

    // Find the last occurrence of start marker (in case of multiple runs)
    let start_pos = contents.rfind(start_marker)?;
    let after_start = &contents[start_pos + start_marker.len()..];

    // Find the end marker after the start
    let end_pos = after_start.find(end_marker)?;
    let json_section = &after_start[..end_pos];

    // The JSON might be on the next line or have log prefixes, so we need to clean it up
    // iOS NSLog format often looks like: "2026-01-20 12:34:56.789 BenchRunner[1234:5678] {"key": "value"}"
    // or just the raw JSON on its own line

    // Try to find valid JSON in the section
    let json_str = extract_json_from_log_section(json_section)?;

    serde_json::from_str::<Value>(&json_str).ok()
}

/// Extract valid JSON from a log section that may contain log prefixes/timestamps.
/// Handles both raw JSON and JSON embedded in log lines.
fn extract_json_from_log_section(section: &str) -> Option<String> {
    // First, try the whole section as-is (trimmed)
    let trimmed = section.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        if serde_json::from_str::<Value>(trimmed).is_ok() {
            return Some(trimmed.to_string());
        }
    }

    // If that didn't work, look for JSON on individual lines
    // This handles cases where NSLog adds timestamps/prefixes
    for line in section.lines() {
        let line = line.trim();

        // Skip empty lines
        if line.is_empty() {
            continue;
        }

        // Look for JSON starting with {
        if let Some(json_start) = line.find('{') {
            let potential_json = &line[json_start..];

            // Try to find the matching closing brace
            // This handles cases like: "timestamp prefix {"key": "value"} suffix"
            if let Some(json) = extract_balanced_json(potential_json) {
                if serde_json::from_str::<Value>(&json).is_ok() {
                    return Some(json);
                }
            }
        }
    }

    // Try concatenating all lines and looking for JSON (for multi-line JSON)
    let all_content: String = section
        .lines()
        .map(|line| {
            // Try to strip common log prefixes (timestamps, process info)
            // iOS format: "2026-01-20 12:34:56.789 AppName[pid:tid] content"
            if let Some(bracket_end) = line.find("] ") {
                &line[bracket_end + 2..]
            } else {
                line.trim()
            }
        })
        .collect::<Vec<_>>()
        .join("");

    if let Some(json_start) = all_content.find('{') {
        let potential_json = &all_content[json_start..];
        if let Some(json) = extract_balanced_json(potential_json) {
            if serde_json::from_str::<Value>(&json).is_ok() {
                return Some(json);
            }
        }
    }

    None
}

/// Extract a balanced JSON object from a string starting with '{'.
/// Returns the JSON substring if balanced braces are found.
fn extract_balanced_json(s: &str) -> Option<String> {
    if !s.starts_with('{') {
        return None;
    }

    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, c) in s.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match c {
            '\\' if in_string => {
                escape_next = true;
            }
            '"' => {
                in_string = !in_string;
            }
            '{' if !in_string => {
                depth += 1;
            }
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[..=i].to_string());
                }
            }
            _ => {}
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
    device_matrix: Option<&Path>,
    device_tags: Vec<String>,
    ios_app: Option<PathBuf>,
    ios_test_suite: Option<PathBuf>,
    local_only: bool,
    release: bool,
) -> Result<RunSpec> {
    if let Some(cfg_path) = config {
        let cfg = load_config(cfg_path)?;
        let matrix = load_device_matrix(&cfg.device_matrix)?;
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
        println!("📦 Auto-packaging iOS artifacts for BrowserStack...");
        let artifacts = package_ios_xcuitest_artifacts(release)?;
        println!("  ✓ IPA: {}", artifacts.app.display());
        println!("  ✓ XCUITest: {}", artifacts.test_suite.display());
        Some(artifacts)
    } else {
        ios_xcuitest
    };

    Ok(RunSpec {
        target,
        function,
        iterations,
        warmup,
        devices: resolved_devices,
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

fn run_ios_build(release: bool) -> Result<(PathBuf, PathBuf)> {
    let root = repo_root()?;
    let crate_name =
        detect_bench_mobile_crate_name(&root).unwrap_or_else(|_| "bench-mobile".to_string());
    let builder = mobench_sdk::builders::IosBuilder::new(&root, crate_name).verbose(true);
    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Ios,
        profile,
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

fn package_ios_xcuitest_artifacts(release: bool) -> Result<IosXcuitestArtifacts> {
    let root = repo_root()?;
    let crate_name =
        detect_bench_mobile_crate_name(&root).unwrap_or_else(|_| "bench-mobile".to_string());
    let builder = mobench_sdk::builders::IosBuilder::new(&root, crate_name).verbose(true);
    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Ios,
        profile,
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
            if let Some(apk_path) = apk {
                if !apk_path.exists() {
                    missing.push(("Android APK".to_string(), apk_path.to_path_buf()));
                }
            }
            if let Some(test_apk_path) = test_apk {
                if !test_apk_path.exists() {
                    missing.push(("Android test APK".to_string(), test_apk_path.to_path_buf()));
                }
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
        username: username.unwrap(),
        access_key: access_key.unwrap(),
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
fn validate_benchmark_function(project_root: &Path, function_name: &str) -> Result<()> {
    // Try to find the benchmark crate
    let crate_name = detect_bench_mobile_crate_name(project_root).ok();

    // Check common crate locations
    let search_dirs = [
        project_root.join("bench-mobile"),
        project_root.join("crates/sample-fns"),
        project_root.to_path_buf(),
    ];

    // Extract the crate name from the function (e.g., "sample_fns::fibonacci" -> "sample_fns")
    let function_crate = function_name.split("::").next().unwrap_or("");

    let mut found_any_benchmarks = false;
    let mut found_function = false;

    for dir in &search_dirs {
        if !dir.join("Cargo.toml").exists() {
            continue;
        }

        // Determine the crate name for this directory
        let dir_crate_name = crate_name.as_deref().unwrap_or(function_crate);

        // Detect all benchmarks in this directory
        let benchmarks = mobench_sdk::codegen::detect_all_benchmarks(dir, dir_crate_name);

        if !benchmarks.is_empty() {
            found_any_benchmarks = true;

            // Check if our function is in the list
            if benchmarks.iter().any(|b| b == function_name) {
                found_function = true;
                break;
            }

            // Also check without crate prefix (in case user specified just the function name)
            let simple_name = function_name.split("::").last().unwrap_or(function_name);
            if benchmarks
                .iter()
                .any(|b| b.ends_with(&format!("::{}", simple_name)))
            {
                found_function = true;
                break;
            }
        }
    }

    if found_any_benchmarks && !found_function {
        // We found benchmarks but not the one requested - this is likely an error
        println!("=== Warning ===");
        println!(
            "  Benchmark function '{}' was not found in the source code.",
            function_name
        );
        println!("  Available benchmarks:");
        for dir in &search_dirs {
            if !dir.join("Cargo.toml").exists() {
                continue;
            }
            let dir_crate_name = crate_name.as_deref().unwrap_or(function_crate);
            let benchmarks = mobench_sdk::codegen::detect_all_benchmarks(dir, dir_crate_name);
            for bench in benchmarks {
                println!("    - {}", bench);
            }
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

fn persist_mobile_spec(spec: &RunSpec, release: bool) -> Result<()> {
    let root = repo_root()?;
    let payload = json!({
        "function": spec.function,
        "iterations": spec.iterations,
        "warmup": spec.warmup,
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
    let mobench_output_dir = root.join("target/mobench");
    let apps_exist =
        mobench_output_dir.join("android").exists() || mobench_output_dir.join("ios").exists();

    if let Err(e) = embed_spec_into_apps(&mobench_output_dir, spec) {
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
fn embed_spec_into_apps(output_dir: &Path, spec: &RunSpec) -> Result<()> {
    let embedded_spec = mobench_sdk::builders::EmbeddedBenchSpec {
        function: spec.function.clone(),
        iterations: spec.iterations,
        warmup: spec.warmup,
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

#[derive(Debug)]
struct SummaryPaths {
    json: PathBuf,
    markdown: PathBuf,
    csv: PathBuf,
}

fn resolve_summary_paths(output: Option<&Path>) -> Result<SummaryPaths> {
    let json = output
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| PathBuf::from("target/mobench/results.json"));
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

const EXIT_REGRESSION: i32 = 2;

fn append_github_step_summary_from_path(path: &Path) -> Result<()> {
    let Ok(summary_path) = env::var("GITHUB_STEP_SUMMARY") else {
        return Ok(());
    };
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading summary markdown {:?}", path))?;
    append_github_step_summary(&contents, &summary_path)
}

fn append_github_step_summary(contents: &str, summary_path: &str) -> Result<()> {
    ensure_parent_dir(Path::new(summary_path))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(summary_path)
        .with_context(|| format!("opening GitHub step summary at {}", summary_path))?;
    file.write_all(contents.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

#[derive(Debug, Clone)]
struct RegressionFinding {
    device: String,
    function: String,
    metric: String,
    delta_pct: f64,
}

fn detect_regressions(report: &CompareReport, threshold_pct: f64) -> Vec<RegressionFinding> {
    let mut findings = Vec::new();
    for row in &report.rows {
        if let Some(delta) = row.median_delta_pct {
            if delta > threshold_pct {
                findings.push(RegressionFinding {
                    device: row.device.clone(),
                    function: row.function.clone(),
                    metric: "median".to_string(),
                    delta_pct: delta,
                });
            }
        }
        if let Some(delta) = row.p95_delta_pct {
            if delta > threshold_pct {
                findings.push(RegressionFinding {
                    device: row.device.clone(),
                    function: row.function.clone(),
                    metric: "p95".to_string(),
                    delta_pct: delta,
                });
            }
        }
    }
    findings
}

fn render_junit_report(summary: &SummaryReport, regressions: &[RegressionFinding]) -> String {
    let mut output = String::new();
    let mut failures_by_case: HashMap<(String, String), Vec<&RegressionFinding>> = HashMap::new();
    for finding in regressions {
        failures_by_case
            .entry((finding.device.clone(), finding.function.clone()))
            .or_default()
            .push(finding);
    }

    let mut total_tests = 0;
    let mut total_failures = 0;

    for device in &summary.device_summaries {
        total_tests += device.benchmarks.len();
        for bench in &device.benchmarks {
            if failures_by_case.contains_key(&(device.device.clone(), bench.function.clone())) {
                total_failures += 1;
            }
        }
    }

    let _ = writeln!(output, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        output,
        r#"<testsuite name="mobench" tests="{}" failures="{}">"#,
        total_tests, total_failures
    );

    for device in &summary.device_summaries {
        for bench in &device.benchmarks {
            let case_name = format!("{}::{}", device.device, bench.function);
            let time_secs = bench
                .median_ns
                .map(|ns| ns as f64 / 1_000_000_000.0)
                .unwrap_or(0.0);
            let _ = writeln!(
                output,
                r#"  <testcase name="{}" classname="{}" time="{:.6}">"#,
                escape_xml(&case_name),
                escape_xml(&device.device),
                time_secs
            );
            if let Some(findings) =
                failures_by_case.get(&(device.device.clone(), bench.function.clone()))
            {
                let mut details = String::new();
                for finding in findings {
                    let _ = writeln!(
                        details,
                        "{} regression: {:+.2}%",
                        finding.metric, finding.delta_pct
                    );
                }
                let _ = writeln!(
                    output,
                    r#"    <failure message="Performance regression">{}</failure>"#,
                    escape_xml(details.trim())
                );
            }
            let _ = writeln!(output, "  </testcase>");
        }
    }

    let _ = writeln!(output, "</testsuite>");
    output
}

fn write_junit_report(
    path: &Path,
    summary: &SummaryReport,
    regressions: &[RegressionFinding],
) -> Result<()> {
    let report = render_junit_report(summary, regressions);
    ensure_parent_dir(path)?;
    write_file(path, report.as_bytes())?;
    println!("Wrote JUnit report to {:?}", path);
    Ok(())
}

fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Print a final summary with all artifact correlation information (C3).
#[allow(dead_code)]
fn print_run_completion_summary(
    summary: &RunSummary,
    paths: &SummaryPaths,
    output_dir: &Path,
) -> Result<()> {
    println!();
    println!("=== Run Completion Summary ===");
    println!();

    // Build ID and platform
    if let Some(ref remote) = summary.remote_run {
        let (build_id, platform) = match remote {
            RemoteRun::Android { build_id, .. } => (build_id, "Android/Espresso"),
            RemoteRun::Ios { build_id, .. } => (build_id, "iOS/XCUITest"),
        };
        println!("BrowserStack Run:");
        println!("  Build ID:    {}", build_id);
        println!("  Platform:    {}", platform);
        println!(
            "  Dashboard:   https://app-automate.browserstack.com/dashboard/v2/builds/{}",
            build_id
        );
        println!();

        // Fetch command for later retrieval
        let target_str = match summary.spec.target {
            MobileTarget::Android => "android",
            MobileTarget::Ios => "ios",
        };
        println!("Fetch Results Later:");
        println!(
            "  cargo mobench fetch --target {} --build-id {} --output-dir ./results",
            target_str, build_id
        );
        println!();
    }

    // Devices tested
    if !summary.spec.devices.is_empty() {
        println!("Devices Tested ({}):", summary.spec.devices.len());
        for device in &summary.spec.devices {
            println!("  - {}", device);
        }
        println!();
    }

    // Results summary by device
    if !summary.summary.device_summaries.is_empty() {
        println!("Results Summary:");
        for device_summary in &summary.summary.device_summaries {
            println!("  Device: {}", device_summary.device);
            for bench in &device_summary.benchmarks {
                let median = bench
                    .median_ns
                    .map(format_duration_smart)
                    .unwrap_or_else(|| "-".to_string());
                let samples = bench.samples;
                println!(
                    "    {} - median: {}, samples: {}",
                    bench.function, median, samples
                );
            }
        }
        println!();
    }

    // Artifact locations
    println!("Output Artifacts:");
    println!("  JSON Summary:     {}", paths.json.display());
    println!("  Markdown Report:  {}", paths.markdown.display());
    if paths.csv.exists() {
        println!("  CSV Data:         {}", paths.csv.display());
    }

    // Build artifacts
    match summary.spec.target {
        MobileTarget::Android => {
            let apk_dir = output_dir.join("android/app/build/outputs/apk");
            if apk_dir.exists() {
                println!("  Android APK:      {}/", apk_dir.display());
            }
        }
        MobileTarget::Ios => {
            let ios_dir = output_dir.join("ios");
            if ios_dir.exists() {
                println!("  iOS Framework:    {}/", ios_dir.display());
            }
        }
    }

    // Bench spec and meta locations
    let spec_path = match summary.spec.target {
        MobileTarget::Android => output_dir.join("android/app/src/main/assets/bench_spec.json"),
        MobileTarget::Ios => {
            output_dir.join("ios/BenchRunner/BenchRunner/Resources/bench_spec.json")
        }
    };
    if spec_path.exists() {
        println!("  Bench Spec:       {}", spec_path.display());
    }

    let meta_path = match summary.spec.target {
        MobileTarget::Android => output_dir.join("android/app/src/main/assets/bench_meta.json"),
        MobileTarget::Ios => {
            output_dir.join("ios/BenchRunner/BenchRunner/Resources/bench_meta.json")
        }
    };
    if meta_path.exists() {
        println!("  Bench Meta:       {}", meta_path.display());
    }

    println!();
    println!("Run completed successfully.");

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

/// Formats a duration in nanoseconds to a human-readable string.
///
/// The function picks the appropriate unit based on the magnitude:
/// - Uses milliseconds (ms) by default
/// - Switches to seconds (s) if the value is >= 1000ms (1 second)
///
/// Examples:
/// - 500_000 ns -> "0.500ms"
/// - 1_500_000 ns -> "1.500ms"
/// - 1_500_000_000 ns -> "1.500s"
fn format_duration_smart(ns: u64) -> String {
    let ms = ns as f64 / 1_000_000.0;
    if ms >= 1000.0 {
        // Convert to seconds
        let secs = ms / 1000.0;
        format!("{:.3}s", secs)
    } else {
        format!("{:.3}ms", ms)
    }
}

fn format_ms(value: Option<u64>) -> String {
    value
        .map(format_duration_smart)
        .unwrap_or_else(|| "-".to_string())
}

fn run_android_build(_ndk_home: &str, release: bool) -> Result<mobench_sdk::BuildResult> {
    let root = repo_root()?;
    let crate_name =
        detect_bench_mobile_crate_name(&root).unwrap_or_else(|_| "bench-mobile".to_string());

    let profile = if release {
        mobench_sdk::BuildProfile::Release
    } else {
        mobench_sdk::BuildProfile::Debug
    };
    let cfg = mobench_sdk::BuildConfig {
        target: mobench_sdk::Target::Android,
        profile,
        incremental: true,
    };
    let builder = mobench_sdk::builders::AndroidBuilder::new(&root, crate_name).verbose(true);
    let result = builder.build(&cfg)?;
    Ok(result)
}

fn load_dotenv() {
    if let Ok(root) = repo_root() {
        let _ = dotenvy::from_path(root.join(".env"));
        let _ = dotenvy::from_path_override(root.join(".env.local"));
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
    progress: bool,
) -> Result<()> {
    // Load config file if present (mobench.toml)
    let config_resolver = config::ConfigResolver::new().unwrap_or_default();

    // Progress mode: simplified output
    if progress {
        let project_root = std::env::current_dir().context("Failed to get current directory")?;
        let crate_name = detect_bench_mobile_crate_name(&project_root)
            .unwrap_or_else(|_| "bench-mobile".to_string());
        let effective_output_dir =
            output_dir.or_else(|| config_resolver.output_dir().map(|p| p.to_path_buf()));

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
                println!("[1/3] Building Rust library...");
                let mut builder =
                    mobench_sdk::builders::AndroidBuilder::new(&project_root, crate_name)
                        .verbose(false)
                        .dry_run(dry_run);
                if let Some(ref dir) = effective_output_dir {
                    builder = builder.output_dir(dir);
                }
                if let Some(ref path) = crate_path {
                    builder = builder.crate_dir(path);
                }
                println!("[2/3] Building Android APK...");
                let result = builder.build(&build_config)?;
                println!("[3/3] Done!");
                if !dry_run {
                    println!("\n\u{2713} APK: {:?}", result.app_path);
                }
            }
            SdkTarget::Ios => {
                println!("[1/3] Building Rust library...");
                let mut builder = mobench_sdk::builders::IosBuilder::new(&project_root, crate_name)
                    .verbose(false)
                    .dry_run(dry_run);
                if let Some(ref dir) = effective_output_dir {
                    builder = builder.output_dir(dir);
                }
                if let Some(ref path) = crate_path {
                    builder = builder.crate_dir(path);
                }
                println!("[2/3] Building iOS xcframework...");
                let result = builder.build(&build_config)?;
                println!("[3/3] Done!");
                if !dry_run {
                    println!("\n\u{2713} Framework: {:?}", result.app_path);
                }
            }
            SdkTarget::Both => {
                println!("[1/5] Building Rust library for Android...");
                let mut android_builder =
                    mobench_sdk::builders::AndroidBuilder::new(&project_root, crate_name.clone())
                        .verbose(false)
                        .dry_run(dry_run);
                if let Some(ref dir) = effective_output_dir {
                    android_builder = android_builder.output_dir(dir);
                }
                if let Some(ref path) = crate_path {
                    android_builder = android_builder.crate_dir(path);
                }
                println!("[2/5] Building Android APK...");
                let android_result = android_builder.build(&build_config)?;

                println!("[3/5] Building Rust library for iOS...");
                let mut ios_builder =
                    mobench_sdk::builders::IosBuilder::new(&project_root, crate_name)
                        .verbose(false)
                        .dry_run(dry_run);
                if let Some(ref dir) = effective_output_dir {
                    ios_builder = ios_builder.output_dir(dir);
                }
                if let Some(ref path) = crate_path {
                    ios_builder = ios_builder.crate_dir(path);
                }
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
    let effective_output_dir =
        output_dir.or_else(|| config_resolver.output_dir().map(|p| p.to_path_buf()));

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
            println!("\nBuilding for Android...");
            println!("  Building Rust library for Android targets...");
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
                println!("\u{2713} Built Android APK");
                println!("\n[checkmark] Android build completed!");
                println!("  APK: {:?}", result.app_path);
            }
        }
        SdkTarget::Ios => {
            println!("\nBuilding for iOS...");
            println!("  Building Rust library for iOS targets...");
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
                println!("\u{2713} Built iOS xcframework");
                println!("\n[checkmark] iOS build completed!");
                println!("  Framework: {:?}", result.app_path);
            }
        }
        SdkTarget::Both => {
            // Build Android
            println!("\nBuilding for Android...");
            println!("  Building Rust library for Android targets...");
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
                println!("\u{2713} Built Android APK");
                println!("\n[checkmark] Android build completed!");
                println!("  APK: {:?}", android_result.app_path);
            }

            // Build iOS
            println!("\nBuilding for iOS...");
            println!("  Building Rust library for iOS targets...");
            let mut ios_builder = mobench_sdk::builders::IosBuilder::new(&project_root, crate_name)
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
        "No benchmark crate found. Expected bench-mobile/Cargo.toml or crates/sample-fns/Cargo.toml under the project root. Run from the project root or set [project].crate in mobench.toml."
    )
}

/// List all discovered benchmark functions
///
/// This uses source code scanning to find `#[benchmark]` functions, which works
/// without requiring a full build. It also falls back to the inventory registry
/// for any benchmarks that may be registered at runtime.
fn cmd_list() -> Result<()> {
    println!("Discovering benchmark functions...\n");

    let project_root = repo_root()?;
    let mut all_benchmarks = Vec::new();

    // Method 1: Source code scanning (works without build)
    let search_dirs = [
        ("bench-mobile", project_root.join("bench-mobile")),
        ("sample-fns", project_root.join("crates/sample-fns")),
        ("ffi-benchmark", project_root.join("crates/ffi-benchmark")),
        ("", project_root.clone()),
    ];

    for (default_crate_name, dir) in &search_dirs {
        if !dir.join("Cargo.toml").exists() {
            continue;
        }
        let crate_name = if default_crate_name.is_empty() {
            if let Ok(name) = get_crate_name_from_cargo_toml(&dir.join("Cargo.toml")) {
                name
            } else {
                continue;
            }
        } else {
            default_crate_name.to_string()
        };
        let benchmarks = mobench_sdk::codegen::detect_all_benchmarks(dir, &crate_name);
        for bench in benchmarks {
            if !all_benchmarks.contains(&bench) {
                all_benchmarks.push(bench);
            }
        }
    }

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
        println!("Searched locations:");
        for (name, dir) in &search_dirs {
            if !name.is_empty() {
                println!("  - {}: {}", name, dir.display());
            }
        }
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
            all_benchmarks.first().unwrap()
        );
    }

    Ok(())
}

fn get_crate_name_from_cargo_toml(cargo_toml: &Path) -> Result<String> {
    let contents = fs::read_to_string(cargo_toml)?;
    let value: toml::Value = toml::from_str(&contents)?;
    let name = value
        .get("package")
        .and_then(|pkg| pkg.get("name"))
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow!("package.name not found in {:?}", cargo_toml))?;
    Ok(name.to_string())
}

/// Package iOS app as IPA for distribution or testing
fn cmd_package_ipa(
    scheme: &str,
    method: IosSigningMethodArg,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    println!("Packaging iOS app as IPA...");
    println!("  Scheme: {}", scheme);
    println!("  Method: {:?}", method);
    if let Some(ref dir) = output_dir {
        println!("  Output: {:?}", dir);
    }

    let project_root = repo_root()?;
    let crate_name = detect_bench_mobile_crate_name(&project_root)
        .unwrap_or_else(|_| "bench-mobile".to_string());

    let mut builder =
        mobench_sdk::builders::IosBuilder::new(&project_root, crate_name).verbose(true);
    if let Some(ref dir) = output_dir {
        builder = builder.output_dir(dir);
    }

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
fn cmd_package_xcuitest(scheme: &str, output_dir: Option<PathBuf>) -> Result<()> {
    println!("Packaging XCUITest runner...");
    println!("  Scheme: {}", scheme);
    if let Some(ref dir) = output_dir {
        println!("  Output: {:?}", dir);
    }

    let project_root = repo_root()?;
    let crate_name = detect_bench_mobile_crate_name(&project_root)
        .unwrap_or_else(|_| "bench-mobile".to_string());

    let mut builder =
        mobench_sdk::builders::IosBuilder::new(&project_root, crate_name).verbose(true);
    if let Some(ref dir) = output_dir {
        builder = builder.output_dir(dir);
    }

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
fn cmd_verify(
    target: Option<SdkTarget>,
    spec_path: Option<PathBuf>,
    check_artifacts: bool,
    smoke_test: bool,
    function: Option<String>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    println!("Verifying benchmark setup...\n");

    let mut checks_passed = 0;
    let mut checks_failed = 0;
    let mut warnings = 0;

    // 1. Check benchmark registry
    print!("  [1/4] Checking benchmark registry... ");
    let benchmarks = mobench_sdk::discover_benchmarks();
    if benchmarks.is_empty() {
        println!("WARNING");
        println!("        No benchmarks found in registry.");
        println!("        This may be expected if benchmarks are in a separate crate.");
        println!(
            "        Tip: Add #[benchmark] attribute to functions and ensure mobench-sdk is linked."
        );
        warnings += 1;
    } else {
        println!("OK ({} benchmark(s) found)", benchmarks.len());
        for bench in &benchmarks {
            println!("        - {}", bench.name);
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
        let project_root = repo_root().unwrap_or_else(|_| PathBuf::from("."));
        let output_base = output_dir
            .clone()
            .unwrap_or_else(|| project_root.join("target/mobench"));
        let default_paths = [
            output_base.join("android/app/src/main/assets/bench_spec.json"),
            output_base.join("ios/BenchRunner/BenchRunner/bench_spec.json"),
            project_root.join("target/mobile-spec/android/bench_spec.json"),
            project_root.join("target/mobile-spec/ios/bench_spec.json"),
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
        let project_root = repo_root().unwrap_or_else(|_| PathBuf::from("."));
        let output_base = output_dir
            .clone()
            .unwrap_or_else(|| project_root.join("target/mobench"));

        let mut artifacts_ok = true;
        let mut artifact_details = Vec::new();

        if let Some(ref t) = target {
            match t {
                SdkTarget::Android | SdkTarget::Both => {
                    let apk_path =
                        output_base.join("android/app/build/outputs/apk/debug/app-debug.apk");
                    let apk_release = output_base
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
                    let jni_base = output_base.join("android/app/src/main/jniLibs");
                    let abis = ["arm64-v8a", "armeabi-v7a", "x86_64"];
                    for abi in abis {
                        let lib_path = jni_base.join(abi).join("libsample_fns.so");
                        if lib_path.exists() {
                            artifact_details.push(format!("JNI lib ({}): OK", abi));
                        }
                    }
                }
                SdkTarget::Ios => {}
            }

            match t {
                SdkTarget::Ios | SdkTarget::Both => {
                    let xcframework = output_base.join("ios/sample_fns.xcframework");
                    if xcframework.exists() {
                        artifact_details.push(format!("iOS xcframework: {:?}", xcframework));
                    } else {
                        artifact_details.push("iOS xcframework: NOT FOUND".to_string());
                        artifacts_ok = false;
                    }

                    let ipa_path = output_base.join("ios/BenchRunner.ipa");
                    if ipa_path.exists() {
                        artifact_details.push(format!("iOS IPA: {:?}", ipa_path));
                    }

                    let xcuitest_path = output_base.join("ios/BenchRunnerUITests.zip");
                    if xcuitest_path.exists() {
                        artifact_details.push(format!("XCUITest runner: {:?}", xcuitest_path));
                    }
                }
                SdkTarget::Android => {}
            }
        } else {
            // Check both platforms by default
            let android_apk = output_base.join("android/app/build/outputs/apk/debug/app-debug.apk");
            let ios_xcframework = output_base.join("ios/sample_fns.xcframework");

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
        if let Some(ref func) = function {
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
        } else if !benchmarks.is_empty() {
            // Use first discovered benchmark
            let func = &benchmarks[0].name;
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
        let iterations = summary
            .get("iterations")
            .and_then(|i| i.as_u64())
            .map(|i| i as u32);
        let warmup = summary
            .get("warmup")
            .and_then(|w| w.as_u64())
            .map(|w| w as u32);

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
            iterations: spec
                .get("iterations")
                .and_then(|i| i.as_u64())
                .map(|i| i as u32),
            warmup: spec
                .get("warmup")
                .and_then(|w| w.as_u64())
                .map(|w| w as u32),
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
            iterations: value
                .get("iterations")
                .and_then(|i| i.as_u64())
                .map(|i| i as u32),
            warmup: value
                .get("warmup")
                .and_then(|w| w.as_u64())
                .map(|w| w as u32),
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
fn print_summary_csv(data: &[SummaryData]) {
    println!(
        "function,device,os_version,sample_count,mean_ns,median_ns,min_ns,max_ns,p95_ns,iterations,warmup"
    );
    for entry in data {
        println!(
            "{},{},{},{},{},{},{},{},{},{},{}",
            entry.function.as_deref().unwrap_or(""),
            entry.device.as_deref().unwrap_or(""),
            entry.os_version.as_deref().unwrap_or(""),
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

/// Check prerequisites for building mobile artifacts.
///
/// This validates that all required tools and configurations are in place
/// before attempting a build.
fn cmd_check(target: SdkTarget, format: CheckOutputFormat) -> Result<()> {
    let checks = collect_prereq_checks(target);
    let issues = collect_issues(&checks);

    match format {
        CheckOutputFormat::Text => print_check_results_text(&checks, &issues),
        CheckOutputFormat::Json => print_check_results_json(&checks)?,
    }

    if issues.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} issue(s) found. Fix them and run 'cargo mobench check --target {:?}' again.",
            issues.len(),
            target
        )
    }
}

fn cmd_doctor(
    target: SdkTarget,
    config_path: Option<&Path>,
    device_matrix_path: Option<&Path>,
    device_tags: Vec<String>,
    browserstack: bool,
    format: CheckOutputFormat,
) -> Result<()> {
    let mut checks = collect_prereq_checks(target);

    let mut config: Option<BenchConfig> = None;
    if let Some(path) = config_path {
        match load_config(path) {
            Ok(cfg) => {
                checks.push(PrereqCheck {
                    name: "Run config".to_string(),
                    passed: true,
                    detail: Some(path.display().to_string()),
                    fix_hint: None,
                });
                config = Some(cfg);
            }
            Err(err) => {
                checks.push(PrereqCheck {
                    name: "Run config".to_string(),
                    passed: false,
                    detail: Some(err.to_string()),
                    fix_hint: Some(format!("Fix or regenerate config at {}", path.display())),
                });
            }
        }
    } else {
        checks.push(PrereqCheck {
            name: "Run config".to_string(),
            passed: true,
            detail: Some("skipped (no --config)".to_string()),
            fix_hint: None,
        });
    }

    let resolved_matrix_path = device_matrix_path
        .map(PathBuf::from)
        .or_else(|| config.as_ref().map(|cfg| cfg.device_matrix.clone()));
    let resolved_tags = if !device_tags.is_empty() {
        Some(device_tags)
    } else {
        config.as_ref().and_then(|cfg| cfg.device_tags.clone())
    };

    if resolved_matrix_path.is_none() && resolved_tags.as_ref().is_some_and(|tags| !tags.is_empty())
    {
        checks.push(PrereqCheck {
            name: "Device matrix".to_string(),
            passed: false,
            detail: Some("device tags provided without a matrix file".to_string()),
            fix_hint: Some(
                "Provide --device-matrix or set device_matrix in the config".to_string(),
            ),
        });
    } else if let Some(path) = resolved_matrix_path.as_deref() {
        match load_device_matrix(path) {
            Ok(matrix) => {
                if let Some(tags) = resolved_tags.as_ref().filter(|tags| !tags.is_empty()) {
                    if let Err(err) = filter_devices_by_tags(matrix.devices, tags) {
                        checks.push(PrereqCheck {
                            name: "Device matrix".to_string(),
                            passed: false,
                            detail: Some(err.to_string()),
                            fix_hint: Some(format!(
                                "Update tags in {} or adjust --device-tags",
                                path.display()
                            )),
                        });
                    } else {
                        checks.push(PrereqCheck {
                            name: "Device matrix".to_string(),
                            passed: true,
                            detail: Some(format!("{} (tags: {})", path.display(), tags.join(", "))),
                            fix_hint: None,
                        });
                    }
                } else {
                    checks.push(PrereqCheck {
                        name: "Device matrix".to_string(),
                        passed: true,
                        detail: Some(path.display().to_string()),
                        fix_hint: None,
                    });
                }
            }
            Err(err) => checks.push(PrereqCheck {
                name: "Device matrix".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some(format!(
                    "Fix or regenerate device matrix at {}",
                    path.display()
                )),
            }),
        }
    } else {
        checks.push(PrereqCheck {
            name: "Device matrix".to_string(),
            passed: true,
            detail: Some("skipped (no --device-matrix)".to_string()),
            fix_hint: None,
        });
    }

    if browserstack {
        let cfg_ref = config.as_ref().map(|cfg| &cfg.browserstack);
        match resolve_browserstack_credentials(cfg_ref) {
            Ok(creds) => checks.push(PrereqCheck {
                name: "BrowserStack credentials".to_string(),
                passed: true,
                detail: Some(format!("user {}", creds.username)),
                fix_hint: None,
            }),
            Err(err) => checks.push(PrereqCheck {
                name: "BrowserStack credentials".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some("Set BROWSERSTACK_USERNAME and BROWSERSTACK_ACCESS_KEY".to_string()),
            }),
        }
    } else {
        checks.push(PrereqCheck {
            name: "BrowserStack credentials".to_string(),
            passed: true,
            detail: Some("skipped (--browserstack=false)".to_string()),
            fix_hint: None,
        });
    }

    let issues = collect_issues(&checks);
    match format {
        CheckOutputFormat::Text => print_check_results_text(&checks, &issues),
        CheckOutputFormat::Json => print_check_results_json(&checks)?,
    }

    if issues.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} issue(s) found. Fix them and rerun 'cargo mobench doctor'.",
            issues.len()
        )
    }
}

fn collect_prereq_checks(target: SdkTarget) -> Vec<PrereqCheck> {
    let mut checks: Vec<PrereqCheck> = Vec::new();
    checks.push(check_cargo());
    checks.push(check_rustup());

    match target {
        SdkTarget::Android => {
            println!("Checking prerequisites for Android...\n");
            checks.push(check_android_ndk_home());
            checks.push(check_cargo_ndk());
            checks.push(check_rust_target("aarch64-linux-android"));
            checks.push(check_rust_target("armv7-linux-androideabi"));
            checks.push(check_rust_target("x86_64-linux-android"));
            checks.push(check_jdk());
        }
        SdkTarget::Ios => {
            println!("Checking prerequisites for iOS...\n");
            checks.push(check_xcode());
            checks.push(check_xcodegen());
            checks.push(check_rust_target("aarch64-apple-ios"));
            checks.push(check_rust_target("aarch64-apple-ios-sim"));
        }
        SdkTarget::Both => {
            println!("Checking prerequisites for Android and iOS...\n");
            checks.push(check_android_ndk_home());
            checks.push(check_cargo_ndk());
            checks.push(check_rust_target("aarch64-linux-android"));
            checks.push(check_rust_target("armv7-linux-androideabi"));
            checks.push(check_rust_target("x86_64-linux-android"));
            checks.push(check_jdk());
            checks.push(check_xcode());
            checks.push(check_xcodegen());
            checks.push(check_rust_target("aarch64-apple-ios"));
            checks.push(check_rust_target("aarch64-apple-ios-sim"));
        }
    }

    checks
}

fn collect_issues(checks: &[PrereqCheck]) -> Vec<String> {
    let mut issues = Vec::new();
    for check in checks {
        if !check.passed {
            if let Some(ref fix) = check.fix_hint {
                issues.push(fix.clone());
            }
        }
    }
    issues
}

#[derive(Debug, Clone, Serialize)]
struct PrereqCheck {
    name: String,
    passed: bool,
    detail: Option<String>,
    fix_hint: Option<String>,
}

fn print_check_results_text(checks: &[PrereqCheck], issues: &[String]) {
    for check in checks {
        let status = if check.passed { "\u{2713}" } else { "\u{2717}" };
        let detail = check.detail.as_deref().unwrap_or("");
        if detail.is_empty() {
            println!("{} {}", status, check.name);
        } else {
            println!("{} {} ({})", status, check.name, detail);
        }
    }

    if !issues.is_empty() {
        println!("\nTo fix:");
        for issue in issues {
            println!("  * {}", issue);
        }
        println!();
        let failed_count = checks.iter().filter(|c| !c.passed).count();
        println!("{} issue(s) found.", failed_count);
    } else {
        println!("\nAll prerequisites satisfied!");
    }
}

fn print_check_results_json(checks: &[PrereqCheck]) -> Result<()> {
    let output = json!({
        "checks": checks,
        "all_passed": checks.iter().all(|c| c.passed),
        "passed_count": checks.iter().filter(|c| c.passed).count(),
        "failed_count": checks.iter().filter(|c| !c.passed).count(),
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn check_cargo() -> PrereqCheck {
    let result = std::process::Command::new("cargo")
        .arg("--version")
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            PrereqCheck {
                name: "cargo installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "cargo installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install Rust: https://rustup.rs".to_string()),
        },
    }
}

fn check_rustup() -> PrereqCheck {
    let result = std::process::Command::new("rustup")
        .arg("--version")
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            PrereqCheck {
                name: "rustup installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "rustup installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install rustup: https://rustup.rs".to_string()),
        },
    }
}

fn check_android_ndk_home() -> PrereqCheck {
    match env::var("ANDROID_NDK_HOME") {
        Ok(path) if !path.is_empty() => {
            let path_exists = Path::new(&path).exists();
            if path_exists {
                PrereqCheck {
                    name: "ANDROID_NDK_HOME set".to_string(),
                    passed: true,
                    detail: Some(path),
                    fix_hint: None,
                }
            } else {
                PrereqCheck {
                    name: "ANDROID_NDK_HOME set".to_string(),
                    passed: false,
                    detail: Some(format!("path does not exist: {}", path)),
                    fix_hint: Some("Set ANDROID_NDK_HOME to a valid NDK path: export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/<version>".to_string()),
                }
            }
        }
        _ => PrereqCheck {
            name: "ANDROID_NDK_HOME set".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some(
                "Set ANDROID_NDK_HOME: export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/<version>"
                    .to_string(),
            ),
        },
    }
}

fn check_cargo_ndk() -> PrereqCheck {
    let result = std::process::Command::new("cargo")
        .args(["ndk", "--version"])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            PrereqCheck {
                name: "cargo-ndk installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "cargo-ndk installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install cargo-ndk: cargo install cargo-ndk".to_string()),
        },
    }
}

fn check_rust_target(target: &str) -> PrereqCheck {
    let result = std::process::Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let installed = String::from_utf8_lossy(&output.stdout);
            let has_target = installed.lines().any(|line| line.trim() == target);
            if has_target {
                PrereqCheck {
                    name: format!("Rust target: {}", target),
                    passed: true,
                    detail: None,
                    fix_hint: None,
                }
            } else {
                PrereqCheck {
                    name: format!("Rust target: {}", target),
                    passed: false,
                    detail: Some("not installed".to_string()),
                    fix_hint: Some(format!("Install target: rustup target add {}", target)),
                }
            }
        }
        _ => PrereqCheck {
            name: format!("Rust target: {}", target),
            passed: false,
            detail: Some("could not check".to_string()),
            fix_hint: Some(format!("Install target: rustup target add {}", target)),
        },
    }
}

fn check_jdk() -> PrereqCheck {
    // Try java -version
    let result = std::process::Command::new("java").arg("-version").output();

    match result {
        Ok(output) => {
            // Java outputs version to stderr
            let version_output = String::from_utf8_lossy(&output.stderr);
            let version_line = version_output.lines().next().unwrap_or("");

            if output.status.success() || !version_line.is_empty() {
                PrereqCheck {
                    name: "JDK installed".to_string(),
                    passed: true,
                    detail: Some(version_line.trim().to_string()),
                    fix_hint: None,
                }
            } else {
                PrereqCheck {
                    name: "JDK installed".to_string(),
                    passed: false,
                    detail: None,
                    fix_hint: Some("Install JDK 17+: brew install openjdk@17".to_string()),
                }
            }
        }
        Err(_) => PrereqCheck {
            name: "JDK installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install JDK 17+: brew install openjdk@17".to_string()),
        },
    }
}

fn check_xcode() -> PrereqCheck {
    let result = std::process::Command::new("xcodebuild")
        .arg("-version")
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            PrereqCheck {
                name: "Xcode installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "Xcode installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some(
                "Install Xcode from the App Store or run: xcode-select --install".to_string(),
            ),
        },
    }
}

fn check_xcodegen() -> PrereqCheck {
    let result = std::process::Command::new("xcodegen")
        .arg("--version")
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            PrereqCheck {
                name: "xcodegen installed".to_string(),
                passed: true,
                detail: Some(version),
                fix_hint: None,
            }
        }
        _ => PrereqCheck {
            name: "xcodegen installed".to_string(),
            passed: false,
            detail: None,
            fix_hint: Some("Install xcodegen: brew install xcodegen".to_string()),
        },
    }
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
            Vec::new(),
            None,
            None,
            false,
            false, // release
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
            Vec::new(),
            None,
            None,
            false,
            false, // release
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
                "mean_ns": 100,
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
        assert_eq!(result.min_ns, Some(80));
        assert_eq!(result.max_ns, Some(120));
        assert!(result.std_dev_ns.is_some());
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
