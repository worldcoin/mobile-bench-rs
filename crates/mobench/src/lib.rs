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
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
mod github;
mod plots;
mod profile;
pub(crate) mod summarize;

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
        #[arg(
            long,
            help = "Project root containing mobench.toml or the Cargo workspace"
        )]
        project_root: Option<PathBuf>,
        #[arg(
            long,
            help = "Path to the benchmark crate directory containing Cargo.toml"
        )]
        crate_path: Option<PathBuf>,
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
        #[arg(long, help = "Baseline summary source (path|url|artifact:<path>)")]
        baseline: Option<String>,
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
    /// Validate run configuration and associated files.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
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
            help = "Project root containing mobench.toml or the Cargo workspace"
        )]
        project_root: Option<PathBuf>,
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
            help = "Project root containing mobench.toml or the Cargo workspace"
        )]
        project_root: Option<PathBuf>,
        #[arg(
            long,
            help = "Path to the benchmark crate directory containing Cargo.toml"
        )]
        crate_path: Option<PathBuf>,
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
            help = "Project root containing mobench.toml or the Cargo workspace"
        )]
        project_root: Option<PathBuf>,
        #[arg(
            long,
            help = "Path to the benchmark crate directory containing Cargo.toml"
        )]
        crate_path: Option<PathBuf>,
        #[arg(
            long,
            help = "Output directory for mobile artifacts (default: target/mobench)"
        )]
        output_dir: Option<PathBuf>,
    },
    /// List all discovered benchmark functions (Phase 1 MVP).
    List {
        #[arg(
            long,
            help = "Project root containing mobench.toml or the Cargo workspace"
        )]
        project_root: Option<PathBuf>,
        #[arg(
            long,
            help = "Path to the benchmark crate directory containing Cargo.toml"
        )]
        crate_path: Option<PathBuf>,
    },
    /// Verify benchmark setup: registry, spec, artifacts, and optional smoke test.
    ///
    /// This command validates:
    /// - Registry has benchmark functions registered
    /// - Spec file exists and is valid (if --spec-path provided)
    /// - Artifacts are present and consistent (if --check-artifacts)
    /// - Runs a local smoke test (if --smoke-test and function is specified)
    Verify {
        #[arg(
            long,
            help = "Project root containing mobench.toml or the Cargo workspace"
        )]
        project_root: Option<PathBuf>,
        #[arg(
            long,
            help = "Path to the benchmark crate directory containing Cargo.toml"
        )]
        crate_path: Option<PathBuf>,
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
        #[command(subcommand)]
        command: Option<DevicesCommand>,
        #[arg(long, value_enum, help = "Filter by platform (android or ios)")]
        platform: Option<DevicePlatform>,
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(long, help = "Validate device specs against available devices")]
        validate: Vec<String>,
    },
    /// Fixture lifecycle helpers for reproducible CI setup.
    Fixture {
        #[command(subcommand)]
        command: FixtureCommand,
    },
    /// Reporting helpers for CI summaries and PR comments.
    Report {
        #[command(subcommand)]
        command: ReportCommand,
    },
    /// Profiling helpers for native profile capture and summary rendering.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
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
    /// Run a full CI benchmark flow with stable output contract.
    Run(CiRunArgs),
    /// Summarize benchmark results with device metrics.
    Summarize(CiSummarizeArgs),
    /// Create a GitHub Check Run with benchmark results.
    CheckRun(CiCheckRunArgs),
}

#[derive(Subcommand, Debug)]
enum DevicesCommand {
    /// Resolve devices from a matrix deterministically for CI usage.
    Resolve {
        #[arg(long, value_enum)]
        platform: DevicePlatform,
        #[arg(long, help = "Device profile/tag to resolve (defaults to `default`)")]
        profile: Option<String>,
        #[arg(
            long,
            help = "Path to run config file (optional source for matrix/tags)"
        )]
        config: Option<PathBuf>,
        #[arg(long, help = "Path to device matrix YAML file")]
        device_matrix: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = CheckOutputFormat::Text)]
        format: CheckOutputFormat,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommand {
    /// Validate bench-config.toml and referenced matrix/settings.
    Validate {
        #[arg(long, default_value = "bench-config.toml")]
        config: PathBuf,
        #[arg(long, value_enum, default_value_t = CheckOutputFormat::Text)]
        format: CheckOutputFormat,
    },
}

#[derive(Subcommand, Debug)]
enum FixtureCommand {
    /// Create starter fixture files for CI runs.
    Init {
        #[arg(long, default_value = "bench-config.toml")]
        config: PathBuf,
        #[arg(long, default_value = "device-matrix.yaml")]
        device_matrix: PathBuf,
        #[arg(long, help = "Overwrite existing fixture files")]
        force: bool,
    },
    /// Build fixture artifacts using existing build commands.
    Build {
        #[arg(long, value_enum, default_value_t = SdkTarget::Both)]
        target: SdkTarget,
        #[arg(long, help = "Build in release mode")]
        release: bool,
        #[arg(long, help = "Output directory for mobile artifacts")]
        output_dir: Option<PathBuf>,
        #[arg(long, help = "Path to benchmark crate")]
        crate_path: Option<PathBuf>,
        #[arg(long, help = "Show simplified step-by-step progress output")]
        progress: bool,
    },
    /// Verify fixture files and optional profile filtering.
    Verify {
        #[arg(long, default_value = "bench-config.toml")]
        config: PathBuf,
        #[arg(long)]
        device_matrix: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = SdkTarget::Both)]
        target: SdkTarget,
        #[arg(long, help = "Device profile/tag to verify")]
        profile: Option<String>,
        #[arg(long, value_enum, default_value_t = CheckOutputFormat::Text)]
        format: CheckOutputFormat,
    },
    /// Compute deterministic fixture cache key from config/toolchain inputs.
    CacheKey {
        #[arg(long, default_value = "bench-config.toml")]
        config: PathBuf,
        #[arg(long)]
        device_matrix: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = SdkTarget::Both)]
        target: SdkTarget,
        #[arg(long, help = "Device profile/tag for keying")]
        profile: Option<String>,
        #[arg(long, value_enum, default_value_t = CheckOutputFormat::Text)]
        format: CheckOutputFormat,
    },
}

#[derive(Subcommand, Debug)]
enum ReportCommand {
    /// Generate markdown summary from standardized output JSON.
    Summarize {
        #[arg(long, default_value = "target/mobench/ci/summary.json")]
        summary: PathBuf,
        #[arg(long, help = "Write markdown output to file")]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = plots::PlotMode::Auto)]
        plots: plots::PlotMode,
    },
    /// Generate/publish sticky GitHub PR comment from summary output.
    Github {
        #[arg(
            long,
            help = "Pull request number (auto-detected from GITHUB_REF if omitted)"
        )]
        pr: Option<String>,
        #[arg(long, default_value = "target/mobench/ci/summary.json")]
        summary: PathBuf,
        #[arg(long, default_value = "<!-- mobench-report -->")]
        marker: String,
        #[arg(long, help = "Publish via GitHub API using GITHUB_TOKEN")]
        publish: bool,
        #[arg(long, help = "Write generated comment body to file")]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum ProfileCommand {
    /// Run a native profiling session and write profile artifacts.
    Run(profile::ProfileRunArgs),
    /// Render markdown or JSON from a normalized profile manifest.
    Summarize(profile::ProfileSummarizeArgs),
}

#[derive(Args, Debug, Clone)]
struct CiRunArgs {
    #[arg(long, value_enum)]
    target: CiTarget,
    #[arg(
        long,
        help = "Path to the benchmark crate directory containing Cargo.toml"
    )]
    crate_path: Option<PathBuf>,
    #[arg(
        long,
        help = "Fully-qualified Rust function to benchmark (single function)"
    )]
    function: Option<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Multiple benchmark functions (comma-separated or JSON array). Runs each in sequence."
    )]
    functions: Vec<String>,
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
    #[arg(long, help = "Baseline summary source (path|url|artifact:<path>)")]
    baseline: Option<String>,
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
    #[arg(
        long,
        default_value = "target/mobench/ci",
        help = "Output directory for CI contract files"
    )]
    output_dir: PathBuf,
    #[arg(long, help = "Metadata: user or actor that requested the run")]
    requested_by: Option<String>,
    #[arg(long, help = "Metadata: pull request number")]
    pr_number: Option<String>,
    #[arg(long, help = "Metadata: original command requested by the caller")]
    request_command: Option<String>,
    #[arg(long, help = "Metadata: git ref/sha for this mobench invocation")]
    mobench_ref: Option<String>,
    #[arg(long, value_enum, default_value_t = plots::PlotMode::Auto)]
    plots: plots::PlotMode,
}

#[derive(Args, Debug, Clone)]
struct CiSummarizeArgs {
    /// BrowserStack build ID to enrich results with device metrics (requires --results-dir).
    #[arg(long)]
    build_id: Option<String>,

    /// Directory containing summary.json/CSV results (offline mode).
    #[arg(long)]
    results_dir: Option<PathBuf>,

    /// Output format: table (terminal), markdown, or json.
    #[arg(long, value_enum, default_value_t = SummarizeFormat::Table)]
    output_format: SummarizeFormat,

    /// Write output to file in addition to stdout.
    #[arg(long)]
    output_file: Option<PathBuf>,

    /// Platform filter (show only one platform).
    #[arg(long, value_enum)]
    platform: Option<MobileTarget>,
}

#[derive(Args, Debug, Clone)]
struct CiCheckRunArgs {
    /// Path to summary JSON with benchmark results.
    #[arg(long, required_unless_present = "results_dir")]
    results: Option<PathBuf>,

    /// Directory containing summary JSON files (processes all).
    #[arg(long, required_unless_present = "results")]
    results_dir: Option<PathBuf>,

    /// GitHub repository (owner/repo format).
    #[arg(long)]
    repo: String,

    /// Git commit SHA to annotate.
    #[arg(long)]
    sha: String,

    /// GitHub App token (from GITHUB_TOKEN env var or actions/create-github-app-token).
    #[arg(long, env = "GITHUB_TOKEN", hide = true)]
    token: String,

    /// Check Run name displayed in the PR.
    #[arg(long, default_value = "Mobench")]
    name: String,

    /// Optional baseline JSON for regression detection.
    #[arg(long)]
    baseline: Option<PathBuf>,

    /// Regression threshold percentage.
    #[arg(long, default_value_t = 5.0)]
    regression_threshold_pct: f64,

    /// File path used in Check Run annotations (relative to repo root).
    #[arg(long, default_value = "src/lib.rs")]
    annotation_path: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SummarizeFormat {
    Table,
    Markdown,
    Json,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CiTarget {
    Android,
    Ios,
    Both,
}

impl CiTarget {
    fn targets(self) -> &'static [MobileTarget] {
        match self {
            CiTarget::Android => &[MobileTarget::Android],
            CiTarget::Ios => &[MobileTarget::Ios],
            CiTarget::Both => &[MobileTarget::Android, MobileTarget::Ios],
        }
    }
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

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ContractErrorCategory {
    Config,
    Preflight,
    Provider,
    Build,
    Benchmark,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Mobile platform target for build/run operations.
pub enum MobileTarget {
    /// Android platform.
    Android,
    /// iOS platform.
    Ios,
}

impl MobileTarget {
    fn as_str(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Ios => "ios",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Android => "Android",
            Self::Ios => "iOS",
        }
    }
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

#[derive(Debug, Clone)]
struct ResolvedProjectLayout {
    project_root: PathBuf,
    crate_dir: PathBuf,
    crate_name: String,
    library_name: String,
    config_path: Option<PathBuf>,
    output_dir: PathBuf,
    default_function: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct ProjectLayoutOptions<'a> {
    start_dir: Option<&'a Path>,
    project_root: Option<&'a Path>,
    crate_path: Option<&'a Path>,
    config_path: Option<&'a Path>,
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
    peak_memory_kb: Option<u64>,
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

pub fn run() -> Result<()> {
    // Load dotenv globally as a baseline for commands that don't resolve a layout
    // (e.g. fetch, doctor, ci run). Layout-aware commands reload from the resolved root.
    load_dotenv_global();
    let cli = Cli::parse();
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
                local_only,
                release,
                cli.dry_run,
            )?;
            let summary_paths = resolve_summary_paths(output.as_deref())?;
            let output_dir = layout.output_dir.clone();

            // Validate device specs early to catch errors before building (C2: Device validation)
            if !spec.devices.is_empty() && !local_only {
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
                        let (xcframework, header) = run_ios_build(&layout, release, cli.dry_run)?;
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
                            if ios_xcuitest.as_ref().is_some_and(|artifacts| {
                                uses_managed_ios_xcuitest_artifacts(&layout, artifacts)
                            }) {
                                println!(
                                    "📦 Packaging iOS BrowserStack artifacts with current bench_spec..."
                                );
                                let packaged = package_ios_xcuitest_artifacts(&layout, release)?;
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
            write_summary(
                &run_summary,
                &summary_paths,
                summary_csv,
                plots::PlotMode::Off,
            )?;

            let mut compare_report = None;
            let mut regression_findings: Vec<RegressionFinding> = Vec::new();
            if let Some(baseline_path) = baseline_compare_path.as_deref() {
                let report = compare_summaries(&baseline_path, &summary_paths.json)?;
                regression_findings = detect_regressions(&report, regression_threshold_pct);
                compare_report = Some(report);
            }
            if let Some(snapshot_path) = baseline_snapshot_path {
                if let Err(err) = fs::remove_file(&snapshot_path) {
                    eprintln!(
                        "Warning: failed to remove baseline snapshot {}: {err}",
                        snapshot_path.display()
                    );
                }
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
            project_root,
            output_dir,
            crate_path,
            progress,
        } => {
            cmd_build(
                target,
                release,
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
                profile::cmd_profile_run(&args)?;
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

fn resolve_project_layout(options: ProjectLayoutOptions<'_>) -> Result<ResolvedProjectLayout> {
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

#[derive(Debug, Serialize)]
struct CiContractMetadata {
    requested_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_number: Option<String>,
    request_command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mobench_ref: Option<String>,
    mobench_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Device input sources used by [`RunRequest`].
pub struct DeviceSelection {
    /// Explicit device names/specs to run against.
    pub devices: Vec<String>,
    /// Optional path to a device matrix YAML file.
    pub device_matrix: Option<PathBuf>,
    /// Optional tag filters applied to the device matrix.
    pub device_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Programmatic request payload for running a mobench benchmark flow.
pub struct RunRequest {
    /// Mobile platform target (`android` or `ios`).
    pub target: MobileTarget,
    /// Fully-qualified benchmark function name.
    pub function: String,
    /// Optional path to the benchmark crate directory.
    pub crate_path: Option<PathBuf>,
    /// Number of benchmark iterations.
    pub iterations: u32,
    /// Number of warmup iterations.
    pub warmup: u32,
    /// Device selection inputs.
    pub device_selection: DeviceSelection,
    /// Optional run configuration file (`bench-config.toml`).
    pub config: Option<PathBuf>,
    /// Optional baseline source (`path|url|artifact:<path>`).
    pub baseline: Option<String>,
    /// Regression threshold percentage used for baseline comparison.
    pub regression_threshold_pct: f64,
    /// Optional JUnit XML output path.
    pub junit: Option<PathBuf>,
    /// When true, skip mobile builds and run local harness only.
    pub local_only: bool,
    /// Build in release mode.
    pub release: bool,
    /// Optional iOS app bundle for BrowserStack XCUITest.
    pub ios_app: Option<PathBuf>,
    /// Optional iOS XCUITest suite package for BrowserStack.
    pub ios_test_suite: Option<PathBuf>,
    /// Fetch BrowserStack artifacts after completion.
    pub fetch: bool,
    /// Output directory for fetched BrowserStack artifacts.
    pub fetch_output_dir: PathBuf,
    /// Poll interval (seconds) when fetching BrowserStack artifacts.
    pub fetch_poll_interval_secs: u64,
    /// Timeout (seconds) when fetching BrowserStack artifacts.
    pub fetch_timeout_secs: u64,
    /// Enable progress-oriented CLI output.
    pub progress: bool,
    /// Output directory for CI contract files.
    pub output_dir: PathBuf,
    /// Plot rendering mode for local markdown summaries.
    pub plots: plots::PlotMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Standardized output file locations produced by a run.
pub struct Report {
    /// Path to JSON summary output.
    pub summary_json: PathBuf,
    /// Path to Markdown summary output.
    pub summary_md: PathBuf,
    /// Path to CSV summary output.
    pub results_csv: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Result of a programmatic mobench run request.
pub struct RunResult {
    /// Platform target executed for this run.
    pub target: MobileTarget,
    /// Generated report file paths.
    pub report: Report,
    /// Exit code from underlying `mobench run` command.
    pub exit_code: i32,
    /// True when regression threshold was exceeded (exit code 2).
    pub regression_detected: bool,
}

/// Executes a [`RunRequest`] by invoking the current `mobench` binary and normalizing outputs.
///
/// This function always writes/normalizes CI output file names in `request.output_dir`:
/// - `summary.json`
/// - `summary.md`
/// - `results.csv`
///
/// Returns [`RunResult`] containing file paths and process exit semantics.
pub fn run_request(request: &RunRequest) -> Result<RunResult> {
    fs::create_dir_all(&request.output_dir)
        .with_context(|| format!("creating output dir {}", request.output_dir.display()))?;

    let summary_json = request.output_dir.join("summary.json");
    let summary_md = request.output_dir.join("summary.md");
    let summary_csv = request.output_dir.join("summary.csv");
    let results_csv = request.output_dir.join("results.csv");

    let mut cmd = std::process::Command::new(
        env::current_exe().context("resolving current mobench executable")?,
    );
    cmd.arg("run")
        .arg("--target")
        .arg(request.target.as_str())
        .arg("--function")
        .arg(&request.function)
        .arg("--iterations")
        .arg(request.iterations.to_string())
        .arg("--warmup")
        .arg(request.warmup.to_string())
        .arg("--ci")
        .arg("--summary-csv")
        .arg("--output")
        .arg(&summary_json)
        .arg("--fetch-output-dir")
        .arg(&request.fetch_output_dir)
        .arg("--fetch-poll-interval-secs")
        .arg(request.fetch_poll_interval_secs.to_string())
        .arg("--fetch-timeout-secs")
        .arg(request.fetch_timeout_secs.to_string())
        .arg("--regression-threshold-pct")
        .arg(request.regression_threshold_pct.to_string());

    for device in &request.device_selection.devices {
        cmd.arg("--devices").arg(device);
    }
    if let Some(path) = &request.device_selection.device_matrix {
        cmd.arg("--device-matrix").arg(path);
    }
    for tag in &request.device_selection.device_tags {
        cmd.arg("--device-tags").arg(tag);
    }
    if let Some(path) = &request.config {
        cmd.arg("--config").arg(path);
    }
    if let Some(path) = &request.baseline {
        cmd.arg("--baseline").arg(path);
    }
    if let Some(path) = &request.junit {
        cmd.arg("--junit").arg(path);
    }
    if request.local_only {
        cmd.arg("--local-only");
    }
    if request.release {
        cmd.arg("--release");
    }
    if let Some(path) = &request.ios_app {
        cmd.arg("--ios-app").arg(path);
    }
    if let Some(path) = &request.ios_test_suite {
        cmd.arg("--ios-test-suite").arg(path);
    }
    if let Some(path) = &request.crate_path {
        cmd.arg("--crate-path").arg(path);
    }
    if request.fetch {
        cmd.arg("--fetch");
    }
    if request.progress {
        cmd.arg("--progress");
    }

    let status = cmd.status().with_context(|| {
        format!(
            "running `cargo mobench run` for target {}",
            request.target.as_str()
        )
    })?;
    let exit_code = status.code().unwrap_or(1);
    if !status.success() && status.code().is_none() {
        bail!("`cargo mobench run` terminated unexpectedly");
    }

    if !summary_json.exists() {
        bail!(
            "expected CI JSON output at {}",
            summary_json.to_string_lossy()
        );
    }
    if !summary_md.exists() {
        bail!(
            "expected CI markdown output at {}",
            summary_md.to_string_lossy()
        );
    }
    if !summary_csv.exists() {
        bail!(
            "expected CI CSV output at {}",
            summary_csv.to_string_lossy()
        );
    }
    if results_csv.exists() {
        fs::remove_file(&results_csv)
            .with_context(|| format!("removing existing {}", results_csv.display()))?;
    }
    fs::rename(&summary_csv, &results_csv).with_context(|| {
        format!(
            "renaming {} to {}",
            summary_csv.display(),
            results_csv.display()
        )
    })?;

    Ok(RunResult {
        target: request.target,
        report: Report {
            summary_json,
            summary_md,
            results_csv,
        },
        exit_code,
        regression_detected: exit_code == EXIT_REGRESSION,
    })
}

fn resolve_ci_functions(args: &CiRunArgs) -> Result<Vec<String>> {
    let mut funcs = args.functions.clone();

    // --function (singular) is sugar for a single-element list
    if let Some(ref f) = args.function {
        if !funcs.contains(f) {
            funcs.insert(0, f.clone());
        }
    }

    // Support JSON array passed as a single element: '["a","b"]'
    if funcs.len() == 1 {
        let trimmed = funcs[0].trim();
        if trimmed.starts_with('[') {
            if let Ok(parsed) = serde_json::from_str::<Vec<String>>(trimmed) {
                return Ok(parsed);
            }
        }
    }

    if funcs.is_empty() {
        bail!("At least one benchmark function is required. Use --function or --functions.");
    }
    Ok(funcs)
}

fn ci_function_slug(function: &str) -> String {
    let mut slug = String::new();
    let mut chars = function.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            ':' if matches!(chars.peek(), Some(':')) => {
                chars.next();
                slug.push('_');
            }
            '_' => slug.push_str("__"),
            '/' => slug.push_str("_slash_"),
            '-' => slug.push('-'),
            ch if ch.is_ascii_alphanumeric() => slug.push(ch),
            ch => slug.push_str(&format!("_x{:02x}", ch as u32)),
        }
    }

    slug
}

fn find_baseline_benchmark<'a>(
    baseline_report: &'a summarize::SummarizeReport,
    platform_name: &str,
    device_name: &str,
    device_os_version: &str,
    benchmark_name: &str,
) -> Option<&'a summarize::BenchmarkResult> {
    baseline_report
        .platforms
        .iter()
        .find(|platform| {
            platform.platform == platform_name
                && summarize::device_names_match(&platform.device.name, device_name)
                && (device_os_version == "unknown"
                    || platform.device.os_version == "unknown"
                    || platform.device.os_version == device_os_version)
        })
        .and_then(|platform| {
            platform
                .benchmarks
                .iter()
                .find(|benchmark| benchmark.name == benchmark_name)
        })
}

fn summary_report_from_value(value: &Value) -> Result<SummaryReport> {
    let summary_value = value
        .get("summary")
        .cloned()
        .unwrap_or_else(|| value.clone());
    serde_json::from_value(summary_value).context("parsing summary report")
}

fn merge_summary_reports(
    target: MobileTarget,
    summaries: &[SummaryReport],
) -> Result<SummaryReport> {
    let first = summaries
        .first()
        .ok_or_else(|| anyhow!("cannot merge empty summary list"))?;

    let latest = summaries
        .iter()
        .max_by_key(|summary| summary.generated_at_unix)
        .unwrap_or(first);

    let mut devices = BTreeSet::new();
    let mut functions = BTreeSet::new();
    let mut device_benchmarks: BTreeMap<String, BTreeMap<String, BenchmarkStats>> = BTreeMap::new();

    for summary in summaries {
        for device in &summary.devices {
            devices.insert(device.clone());
        }
        functions.insert(summary.function.clone());

        for device_summary in &summary.device_summaries {
            let benchmark_map = device_benchmarks
                .entry(device_summary.device.clone())
                .or_default();
            for benchmark in &device_summary.benchmarks {
                benchmark_map.insert(benchmark.function.clone(), benchmark.clone());
            }
        }
    }

    let function = if functions.len() == 1 {
        functions
            .into_iter()
            .next()
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        "multiple".to_string()
    };

    let device_summaries = device_benchmarks
        .into_iter()
        .map(|(device, benchmarks)| DeviceSummary {
            device,
            benchmarks: benchmarks.into_values().collect(),
        })
        .collect();

    Ok(SummaryReport {
        generated_at: latest.generated_at.clone(),
        generated_at_unix: latest.generated_at_unix,
        target,
        function,
        iterations: first.iterations,
        warmup: first.warmup,
        devices: devices.into_iter().collect(),
        device_summaries,
    })
}

fn merge_ci_target_runs(
    target: MobileTarget,
    function_runs: &BTreeMap<String, Value>,
) -> Result<Value> {
    let summaries = function_runs
        .values()
        .map(summary_report_from_value)
        .collect::<Result<Vec<_>>>()?;
    let merged_summary = merge_summary_reports(target, &summaries)?;

    Ok(json!({
        "summary": merged_summary,
        "functions": function_runs
    }))
}

fn root_summary_from_merged_targets(targets: &BTreeMap<String, Value>) -> Option<Value> {
    if targets.len() != 1 {
        return None;
    }

    targets
        .values()
        .next()
        .and_then(|entry| entry.get("summary").cloned())
}

fn cmd_ci_run(args: CiRunArgs) -> Result<()> {
    let all_functions = resolve_ci_functions(&args)?;

    fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("creating ci output dir {}", args.output_dir.display()))?;
    let metadata = ci_metadata_from_args(&args);
    let targets = args.target.targets();

    if targets.len() == 1 && all_functions.len() == 1 {
        // Fast path: single target, single function — original behavior
        let target = targets[0];
        let mut single_args = args.clone();
        single_args.function = Some(all_functions[0].clone());
        let exit_code = cmd_ci_run_single(&single_args, target, &args.output_dir, &metadata)?;

        let summary_json = args.output_dir.join("summary.json");
        let summary_md = args.output_dir.join("summary.md");
        let results_csv = args.output_dir.join("results.csv");
        println!("CI outputs ready:");
        println!("  - {}", summary_json.display());
        println!("  - {}", summary_md.display());
        println!("  - {}", results_csv.display());

        if exit_code == EXIT_REGRESSION {
            std::process::exit(EXIT_REGRESSION);
        }
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
        return Ok(());
    }

    let mut regression_detected = false;
    let mut target_runs: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut target_outputs: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();

    for target in targets {
        let target_value = *target;
        for func in &all_functions {
            let slug = ci_function_slug(func);
            let target_dir = if all_functions.len() == 1 {
                args.output_dir.join(target_value.as_str())
            } else {
                args.output_dir.join(target_value.as_str()).join(&slug)
            };
            fs::create_dir_all(&target_dir)
                .with_context(|| format!("creating target output dir {}", target_dir.display()))?;
            let mut func_args = args.clone();
            func_args.function = Some(func.clone());
            let exit_code = cmd_ci_run_single(&func_args, target_value, &target_dir, &metadata)?;
            if exit_code == EXIT_REGRESSION {
                regression_detected = true;
            } else if exit_code != 0 {
                std::process::exit(exit_code);
            }

            let summary_json = target_dir.join("summary.json");
            let summary_md = target_dir.join("summary.md");
            let results_csv = target_dir.join("results.csv");

            let summary_text = fs::read_to_string(&summary_json)
                .with_context(|| format!("reading {}", summary_json.display()))?;
            let summary_value: Value = serde_json::from_str(&summary_text)
                .with_context(|| format!("parsing {}", summary_json.display()))?;
            target_runs
                .entry(target_value.as_str().to_string())
                .or_default()
                .insert(slug.clone(), summary_value);
            target_outputs
                .entry(target_value.as_str().to_string())
                .or_default()
                .insert(
                    slug,
                    json!({
                    "summary_json": summary_json.display().to_string(),
                    "summary_md": summary_md.display().to_string(),
                    "results_csv": results_csv.display().to_string(),
                    }),
                );
        } // end for func
    } // end for target

    let mut merged_targets = BTreeMap::new();
    for target in targets {
        let target_value = *target;
        let target_key = target_value.as_str().to_string();
        let runs = target_runs
            .get(&target_key)
            .ok_or_else(|| anyhow!("missing merged runs for target `{target_key}`"))?;
        merged_targets.insert(target_key, merge_ci_target_runs(target_value, runs)?);
    }

    let root_summary_json = args.output_dir.join("summary.json");
    let root_summary_md = args.output_dir.join("summary.md");
    let root_results_csv = args.output_dir.join("results.csv");

    let mut merged_csv_rows = Vec::new();
    let mut merged_header: Option<String> = None;

    for (target_name, entry) in &merged_targets {
        let summary = summary_report_from_value(entry)?;
        let csv = render_csv_summary(&summary);
        let mut lines = csv.lines();
        if let Some(header) = lines.next()
            && merged_header.is_none()
        {
            merged_header = Some(format!("target,{header}"));
        }
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            merged_csv_rows.push(format!("{target_name},{line}"));
        }
    }

    let mut merged_csv = String::new();
    if let Some(header) = merged_header {
        merged_csv.push_str(&header);
        merged_csv.push('\n');
    }
    for row in merged_csv_rows {
        merged_csv.push_str(&row);
        merged_csv.push('\n');
    }
    write_file(&root_results_csv, merged_csv.as_bytes())?;

    let root_ci_value = json!({
        "metadata": metadata,
        "outputs": {
            "summary_json": root_summary_json.display().to_string(),
            "summary_md": root_summary_md.display().to_string(),
            "results_csv": root_results_csv.display().to_string(),
        },
        "targets": target_outputs
            .into_iter()
            .map(|(target, functions)| (target, json!({ "functions": functions })))
            .collect::<BTreeMap<_, _>>()
    });
    let mut merged_summary = json!({
        "targets": merged_targets,
        "ci": root_ci_value
    });
    if let Some(summary) = merged_summary
        .get("targets")
        .and_then(|targets| targets.as_object())
        .map(|targets| {
            targets
                .iter()
                .map(|(target, value)| (target.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .and_then(|targets| root_summary_from_merged_targets(&targets))
        && let Some(obj) = merged_summary.as_object_mut()
    {
        obj.insert("summary".to_string(), summary);
    }
    write_file(
        &root_summary_json,
        serde_json::to_string_pretty(&merged_summary)?.as_bytes(),
    )?;
    let merged_markdown = render_summary_markdown_from_output_with_plots(
        &merged_summary,
        &args.output_dir,
        args.plots,
    )?;
    write_file(&root_summary_md, merged_markdown.as_bytes())?;

    println!("CI outputs ready:");
    println!("  - {}", root_summary_json.display());
    println!("  - {}", root_summary_md.display());
    println!("  - {}", root_results_csv.display());

    if regression_detected {
        std::process::exit(EXIT_REGRESSION);
    }
    Ok(())
}

fn ci_metadata_from_args(args: &CiRunArgs) -> CiContractMetadata {
    CiContractMetadata {
        requested_by: args
            .requested_by
            .clone()
            .or_else(|| ci_env(&["MOBENCH_REQUESTED_BY", "GITHUB_ACTOR"]))
            .unwrap_or_else(|| "unknown".to_string()),
        pr_number: args.pr_number.clone().or_else(|| {
            ci_env(&[
                "MOBENCH_PR_NUMBER",
                "PR_NUMBER",
                "GITHUB_PR_NUMBER",
                "GITHUB_PULL_REQUEST_NUMBER",
            ])
            .or_else(infer_pr_number_from_github_ref)
        }),
        request_command: args.request_command.clone().unwrap_or_else(|| {
            let argv: Vec<String> = env::args().collect();
            if argv.is_empty() {
                "cargo mobench ci run".to_string()
            } else {
                argv.join(" ")
            }
        }),
        mobench_ref: args
            .mobench_ref
            .clone()
            .or_else(|| ci_env(&["MOBENCH_REF", "GITHUB_SHA", "GITHUB_REF"])),
        mobench_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn cmd_ci_summarize(args: CiSummarizeArgs) -> Result<()> {
    if args.build_id.is_none() && args.results_dir.is_none() {
        anyhow::bail!("Either --build-id or --results-dir must be provided");
    }

    // Load the report from offline results
    let mut report = if let Some(ref dir) = args.results_dir {
        summarize::load_results_dir(dir)?
    } else {
        // If only build_id, we need results_dir too for now
        anyhow::bail!(
            "--results-dir is required. Use --build-id alongside --results-dir to enrich offline results with BrowserStack metrics."
        );
    };

    // Enrich with BrowserStack data if build_id provided
    if let Some(ref build_id) = args.build_id {
        match resolve_browserstack_credentials(None) {
            Ok(creds) => {
                match BrowserStackClient::new(
                    BrowserStackAuth {
                        username: creds.username,
                        access_key: creds.access_key,
                    },
                    creds.project,
                ) {
                    Ok(client) => {
                        // Try iOS first, then Android
                        let build_summary = client
                            .get_build_summary(build_id, "ios")
                            .or_else(|_| client.get_build_summary(build_id, "android"));

                        match build_summary {
                            Ok(summary) => {
                                summarize::enrich_with_browserstack(&mut report, &summary)
                            }
                            Err(e) => {
                                eprintln!("Warning: could not fetch BrowserStack data: {e}")
                            }
                        }
                    }
                    Err(e) => eprintln!("Warning: could not create BrowserStack client: {e}"),
                }
            }
            Err(e) => eprintln!("Warning: BrowserStack credentials not available: {e}"),
        }
    }

    // Filter by platform if requested
    if let Some(platform) = &args.platform {
        let target = platform.as_str();
        report.platforms.retain(|p| p.platform == target);
    }

    // Render output
    let output = match args.output_format {
        SummarizeFormat::Table => summarize::render_table(&report),
        SummarizeFormat::Markdown => summarize::render_markdown(&report),
        SummarizeFormat::Json => summarize::render_json(&report)?,
    };

    println!("{output}");

    // Optionally write to file
    if let Some(ref path) = args.output_file {
        std::fs::write(path, &output)
            .with_context(|| format!("Failed to write output to {}", path.display()))?;
        eprintln!("Output written to {}", path.display());
    }

    Ok(())
}

fn cmd_ci_check_run(args: CiCheckRunArgs) -> Result<()> {
    // Load results
    let report = if let Some(ref dir) = args.results_dir {
        summarize::load_results_dir(dir)?
    } else if let Some(ref path) = args.results {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&content)?;
        summarize::parse_summary_value(&value)?
    } else {
        anyhow::bail!("Either --results or --results-dir must be provided");
    };

    // Generate markdown summary
    let summary_md = summarize::render_markdown(&report);

    // Check for regressions if baseline provided
    let mut annotations = Vec::new();
    let mut has_regression = false;

    if let Some(baseline_path) = &args.baseline {
        let baseline_content = std::fs::read_to_string(baseline_path)
            .with_context(|| format!("Failed to read baseline {}", baseline_path.display()))?;
        let baseline_value: serde_json::Value = serde_json::from_str(&baseline_content)?;
        let baseline_report = summarize::parse_summary_value(&baseline_value)?;

        for platform in &report.platforms {
            for bench in &platform.benchmarks {
                let baseline_bench = find_baseline_benchmark(
                    &baseline_report,
                    &platform.platform,
                    &platform.device.name,
                    &platform.device.os_version,
                    &bench.name,
                );

                if let Some(base) = baseline_bench {
                    if base.timing.avg_ms > 0.0 {
                        let pct_change =
                            (bench.timing.avg_ms - base.timing.avg_ms) / base.timing.avg_ms * 100.0;

                        if pct_change > args.regression_threshold_pct {
                            has_regression = true;
                            let line = annotations.len() as u32 + 1;
                            annotations.push(github::CheckRunAnnotation {
                                path: args.annotation_path.clone(),
                                start_line: line,
                                end_line: line,
                                annotation_level: "warning".to_string(),
                                message: format!(
                                    "{} regressed {pct_change:+.1}% ({:.1}ms \u{2192} {:.1}ms)",
                                    bench.label, base.timing.avg_ms, bench.timing.avg_ms
                                ),
                                title: format!("Regression: {}", bench.label),
                            });
                        }
                    }
                }
            }
        }
    }

    let conclusion = if has_regression { "failure" } else { "success" };

    let bench_count: usize = report.platforms.iter().map(|p| p.benchmarks.len()).sum();
    let title = if has_regression {
        format!(
            "{bench_count} benchmarks \u{2014} {} regressed",
            annotations.len()
        )
    } else {
        format!("{bench_count} benchmarks passed")
    };

    let client = github::GitHubClient::new(args.token)?;
    let result = client.create_check_run(
        &args.repo,
        &args.sha,
        &args.name,
        conclusion,
        &title,
        &summary_md,
        annotations,
    )?;

    eprintln!(
        "Check Run created: conclusion={}, annotations={}",
        result.conclusion, result.annotations_count
    );

    Ok(())
}

fn cmd_ci_run_single(
    args: &CiRunArgs,
    target: MobileTarget,
    output_dir: &Path,
    metadata: &CiContractMetadata,
) -> Result<i32> {
    let default_baseline_path = previous_baseline_path(output_dir);
    let baseline_source = args.baseline.clone().or_else(|| {
        if default_baseline_path.exists() {
            Some(default_baseline_path.display().to_string())
        } else {
            None
        }
    });

    let result = run_request(&RunRequest {
        target,
        function: args.function.clone().unwrap_or_default(),
        crate_path: args.crate_path.clone(),
        iterations: args.iterations,
        warmup: args.warmup,
        device_selection: DeviceSelection {
            devices: args.devices.clone(),
            device_matrix: args.device_matrix.clone(),
            device_tags: args.device_tags.clone(),
        },
        config: args.config.clone(),
        baseline: baseline_source,
        regression_threshold_pct: args.regression_threshold_pct,
        junit: args.junit.clone(),
        local_only: args.local_only,
        release: args.release,
        ios_app: args.ios_app.clone(),
        ios_test_suite: args.ios_test_suite.clone(),
        fetch: args.fetch,
        fetch_output_dir: args.fetch_output_dir.clone(),
        fetch_poll_interval_secs: args.fetch_poll_interval_secs,
        fetch_timeout_secs: args.fetch_timeout_secs,
        progress: args.progress,
        output_dir: output_dir.to_path_buf(),
        plots: args.plots,
    })?;

    let summary_json = result.report.summary_json;
    let summary_md = result.report.summary_md;
    let results_csv = result.report.results_csv;

    let summary_text = fs::read_to_string(&summary_json)
        .with_context(|| format!("reading {}", summary_json.display()))?;
    let mut summary_value: Value = serde_json::from_str(&summary_text)
        .with_context(|| format!("parsing {}", summary_json.display()))?;
    let ci_value = json!({
        "metadata": metadata,
        "outputs": {
            "summary_json": summary_json.display().to_string(),
            "summary_md": summary_md.display().to_string(),
            "results_csv": results_csv.display().to_string(),
        },
        "target": target.as_str()
    });
    if let Some(obj) = summary_value.as_object_mut() {
        obj.insert("ci".to_string(), ci_value);
    } else {
        summary_value = json!({
            "run_summary": summary_value,
            "ci": ci_value
        });
    }
    let rendered = serde_json::to_string_pretty(&summary_value)?;
    write_file(&summary_json, rendered.as_bytes())?;
    fs::copy(&summary_json, &default_baseline_path).with_context(|| {
        format!(
            "writing previous baseline snapshot to {}",
            default_baseline_path.display()
        )
    })?;

    Ok(result.exit_code)
}

fn previous_baseline_path(output_dir: &Path) -> PathBuf {
    output_dir.join(".previous-summary.json")
}

fn ci_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        env::var(key).ok().and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
    })
}

fn infer_pr_number_from_github_ref() -> Option<String> {
    let github_ref = env::var("GITHUB_REF").ok()?;
    parse_pr_number_from_ref(&github_ref)
}

fn parse_pr_number_from_ref(github_ref: &str) -> Option<String> {
    let parts: Vec<&str> = github_ref.split('/').collect();
    if parts.len() >= 4 && parts[0] == "refs" && parts[1] == "pull" {
        let pr = parts[2].trim();
        if !pr.is_empty() {
            return Some(pr.to_string());
        }
    }
    None
}

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

        let mut downloaded_texts = BTreeMap::new();
        for (key, url) in extract_url_fields(&session_json) {
            let file_name = filename_for_url(&key, &url);
            let dest = session_dir.join(file_name);
            if let Err(err) = client.download_url(&url, &dest) {
                println!("Skipping download for {key}: {err}");
                continue;
            }
            if let Ok(contents) = fs::read_to_string(&dest) {
                downloaded_texts.insert(url, contents);
            }
        }

        if let Ok((bench_results, _)) =
            client.extract_results_from_session_artifacts(&session_json, |url| {
                downloaded_texts
                    .get(url)
                    .cloned()
                    .ok_or_else(|| anyhow!("artifact {url} was not downloaded as text"))
            })
        {
            let report = if bench_results.len() == 1 {
                bench_results.into_iter().next().unwrap_or(Value::Null)
            } else {
                Value::Array(bench_results)
            };
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
    layout: &ResolvedProjectLayout,
    config: Option<&Path>,
    device_matrix: Option<&Path>,
    device_tags: Vec<String>,
    ios_app: Option<PathBuf>,
    ios_test_suite: Option<PathBuf>,
    local_only: bool,
    _release: bool,
    dry_run: bool,
) -> Result<RunSpec> {
    if let Some(cfg_path) = config {
        let cfg = load_config(cfg_path)?;
        let matrix_path = device_matrix.unwrap_or(cfg.device_matrix.as_path());
        let matrix = load_device_matrix(matrix_path)?;
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

fn run_ios_build(
    layout: &ResolvedProjectLayout,
    release: bool,
    dry_run: bool,
) -> Result<(PathBuf, PathBuf)> {
    let builder =
        mobench_sdk::builders::IosBuilder::new(&layout.project_root, layout.crate_name.clone())
            .verbose(true)
            .dry_run(dry_run)
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
    };
    let result = builder.build(&cfg)?;
    let header = layout
        .output_dir
        .join("ios/include")
        .join(format!("{}.h", layout.library_name));
    Ok((result.app_path, header))
}

fn package_ios_xcuitest_artifacts(
    layout: &ResolvedProjectLayout,
    release: bool,
) -> Result<IosXcuitestArtifacts> {
    let builder =
        mobench_sdk::builders::IosBuilder::new(&layout.project_root, layout.crate_name.clone())
            .verbose(true)
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
fn validate_benchmark_function(layout: &ResolvedProjectLayout, function_name: &str) -> Result<()> {
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

fn persist_mobile_spec(
    layout: &ResolvedProjectLayout,
    spec: &RunSpec,
    release: bool,
) -> Result<()> {
    let root = &layout.project_root;
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
    let mobench_output_dir = layout.output_dir.clone();
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
            let perf_metrics = run_summary
                .performance_metrics
                .as_ref()
                .and_then(|metrics| metrics.get(device));
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
                    resource_usage: extract_benchmark_resource_usage(entry, perf_metrics),
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

fn write_summary(
    summary: &RunSummary,
    paths: &SummaryPaths,
    summary_csv: bool,
    plot_mode: plots::PlotMode,
) -> Result<()> {
    let json = serde_json::to_string_pretty(summary)?;
    ensure_parent_dir(&paths.json)?;
    write_file(&paths.json, json.as_bytes())?;
    println!("Wrote run summary to {:?}", paths.json);

    let summary_value = serde_json::to_value(summary).context("serializing run summary")?;
    let markdown_dir = paths.markdown.parent().unwrap_or_else(|| Path::new("."));
    let markdown =
        render_summary_markdown_from_output_with_plots(&summary_value, markdown_dir, plot_mode)?;
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

#[derive(Debug, Serialize)]
struct CompareReport {
    baseline: PathBuf,
    candidate: PathBuf,
    rows: Vec<CompareRow>,
}

#[derive(Debug, Serialize)]
struct CompareRow {
    device: String,
    function: String,
    baseline_median_ns: Option<u64>,
    candidate_median_ns: Option<u64>,
    median_delta_pct: Option<f64>,
    median_label: String,
    baseline_p95_ns: Option<u64>,
    candidate_p95_ns: Option<u64>,
    p95_delta_pct: Option<f64>,
    p95_label: String,
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
                median_label: delta_label(median_delta, 0.0).to_string(),
                baseline_p95_ns: baseline_p95,
                candidate_p95_ns: candidate_p95,
                p95_delta_pct: p95_delta,
                p95_label: delta_label(p95_delta, 0.0).to_string(),
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

fn delta_label(delta: Option<f64>, threshold_pct: f64) -> &'static str {
    match delta {
        Some(value) if value >= threshold_pct => "regressed",
        Some(value) if value <= -threshold_pct => "improved",
        Some(_) => "neutral",
        None => "neutral",
    }
}

fn resolve_baseline_source(source: &str) -> Result<PathBuf> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        bail!("config_error: baseline source is empty");
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let root = repo_root()?;
        let baseline_dir = root.join("target/mobench/baselines");
        fs::create_dir_all(&baseline_dir)?;
        let mut hasher = Sha256::new();
        hasher.update(trimmed.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        let baseline_path = baseline_dir.join(format!("{hash}.json"));

        let response = reqwest::blocking::Client::new()
            .get(trimmed)
            .send()
            .with_context(|| format!("provider_error: downloading baseline URL {trimmed}"))?
            .error_for_status()
            .with_context(|| format!("provider_error: HTTP error for baseline URL {trimmed}"))?;
        let bytes = response
            .bytes()
            .context("provider_error: reading baseline body")?;
        write_file(&baseline_path, bytes.as_ref())?;
        return Ok(baseline_path);
    }

    if let Some(artifact_ref) = trimmed.strip_prefix("artifact:") {
        return resolve_artifact_baseline(artifact_ref.trim());
    }

    Ok(PathBuf::from(trimmed))
}

fn normalized_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .context("resolving current directory for baseline path comparison")?
            .join(path)
    };
    Ok(fs::canonicalize(&absolute).unwrap_or(absolute))
}

fn paths_point_to_same_file(lhs: &Path, rhs: &Path) -> Result<bool> {
    Ok(normalized_path(lhs)? == normalized_path(rhs)?)
}

fn snapshot_baseline_for_compare(path: &Path) -> Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_nanos();
    let snapshot_path = env::temp_dir().join(format!("mobench-baseline-{stamp}.json"));
    fs::copy(path, &snapshot_path).with_context(|| {
        format!(
            "copying baseline snapshot from {} to {}",
            path.display(),
            snapshot_path.display()
        )
    })?;
    Ok(snapshot_path)
}

fn resolve_artifact_baseline(reference: &str) -> Result<PathBuf> {
    if reference.is_empty() {
        bail!("config_error: baseline artifact reference is empty");
    }
    let root = repo_root()?;
    let mut candidates = vec![
        PathBuf::from(reference),
        root.join(reference),
        root.join("target/mobench/ci").join(reference),
    ];
    let artifact_path = root.join("target/mobench/ci").join(reference);
    if artifact_path.is_dir() {
        candidates.push(artifact_path.join("summary.json"));
    }

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    bail!(
        "config_error: baseline artifact `{}` not found (tried path and target/mobench/ci)",
        reference
    )
}

fn inject_compare_into_summary(
    summary_json: &Path,
    report: &CompareReport,
    threshold_pct: f64,
    baseline_source: Option<&str>,
) -> Result<()> {
    let summary_text =
        fs::read_to_string(summary_json).with_context(|| format!("reading {:?}", summary_json))?;
    let mut summary_value: Value = serde_json::from_str(&summary_text)
        .with_context(|| format!("parsing {:?}", summary_json))?;

    let compare_value = json!({
        "baseline": report.baseline.display().to_string(),
        "baseline_source": baseline_source,
        "candidate": report.candidate.display().to_string(),
        "threshold_pct": threshold_pct,
        "rows": report.rows.iter().map(|row| json!({
            "device": row.device,
            "function": row.function,
            "baseline_median_ns": row.baseline_median_ns,
            "candidate_median_ns": row.candidate_median_ns,
            "median_delta_pct": row.median_delta_pct,
            "median_label": delta_label(row.median_delta_pct, threshold_pct),
            "baseline_p95_ns": row.baseline_p95_ns,
            "candidate_p95_ns": row.candidate_p95_ns,
            "p95_delta_pct": row.p95_delta_pct,
            "p95_label": delta_label(row.p95_delta_pct, threshold_pct),
        })).collect::<Vec<_>>()
    });

    if let Some(obj) = summary_value.as_object_mut() {
        obj.insert("comparison".to_string(), compare_value);
    }
    write_file(
        summary_json,
        serde_json::to_string_pretty(&summary_value)?.as_bytes(),
    )?;
    Ok(())
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
        "| Device | Function | Median (base ms) | Median (cand ms) | Median Δ% | Median Label | P95 (base ms) | P95 (cand ms) | P95 Δ% | P95 Label |"
    );
    let _ = writeln!(
        output,
        "| --- | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | --- |"
    );
    for row in &report.rows {
        let _ = writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            row.device,
            row.function,
            format_ms(row.baseline_median_ns),
            format_ms(row.candidate_median_ns),
            format_delta(row.median_delta_pct),
            row.median_label,
            format_ms(row.baseline_p95_ns),
            format_ms(row.candidate_p95_ns),
            format_delta(row.p95_delta_pct),
            row.p95_label
        );
    }
    output
}

#[derive(Debug, Deserialize)]
struct GitHubIssueComment {
    id: u64,
    body: String,
}

fn cmd_report_summarize(
    summary_path: &Path,
    output: Option<&Path>,
    plot_mode: plots::PlotMode,
) -> Result<String> {
    let contents = fs::read_to_string(summary_path)
        .with_context(|| format!("reading summary file {}", summary_path.display()))?;
    let value: Value = serde_json::from_str(&contents)
        .with_context(|| format!("parsing summary file {}", summary_path.display()))?;
    let markdown = render_summary_markdown_from_output_with_plots(
        &value,
        &summary_markdown_output_dir(summary_path, output),
        plot_mode,
    )?;

    if let Some(path) = output {
        ensure_parent_dir(path)?;
        write_file(path, markdown.as_bytes())?;
        println!("Wrote report summary markdown to {}", path.display());
    } else {
        println!("{markdown}");
    }

    Ok(markdown)
}

fn cmd_report_github(
    pr: Option<String>,
    summary_path: &Path,
    marker: &str,
    publish: bool,
    output: Option<&Path>,
) -> Result<()> {
    let contents = fs::read_to_string(summary_path)
        .with_context(|| format!("reading summary file {}", summary_path.display()))?;
    let value: Value = serde_json::from_str(&contents)
        .with_context(|| format!("parsing summary file {}", summary_path.display()))?;
    let markdown = render_summary_markdown_from_output(&value)?;
    let comment_body = format!("{marker}\n\n{markdown}");

    if let Some(path) = output {
        ensure_parent_dir(path)?;
        write_file(path, comment_body.as_bytes())?;
        println!("Wrote GitHub report body to {}", path.display());
    } else if !publish {
        println!("{comment_body}");
    }

    if publish {
        let pr_number = pr
            .or_else(|| ci_env(&["MOBENCH_PR_NUMBER", "PR_NUMBER"]))
            .or_else(infer_pr_number_from_github_ref)
            .ok_or_else(|| anyhow!("provider_error: missing PR number (`--pr` or GITHUB_REF)"))?;
        upsert_github_pr_comment(&pr_number, marker, &comment_body)?;
        println!("Published sticky PR comment for PR #{}", pr_number);
    }

    Ok(())
}

fn render_summary_markdown_from_output(value: &Value) -> Result<String> {
    if let Some(summary) = value.get("summary") {
        let parsed: SummaryReport =
            serde_json::from_value(summary.clone()).context("parsing summary report")?;
        return Ok(render_markdown_summary(&parsed));
    }

    if let Some(targets) = value.get("targets").and_then(|v| v.as_object()) {
        let mut target_names: Vec<String> = targets.keys().cloned().collect();
        target_names.sort();

        let mut sections = Vec::new();
        for name in target_names {
            let Some(entry) = targets.get(&name) else {
                continue;
            };
            let summary_value = entry
                .get("summary")
                .cloned()
                .unwrap_or_else(|| entry.clone());
            let parsed: SummaryReport =
                serde_json::from_value(summary_value).with_context(|| {
                    format!("parsing summary report for target `{name}` in merged output")
                })?;
            sections.push(format!("## {name}\n\n{}", render_markdown_summary(&parsed)));
        }
        if !sections.is_empty() {
            return Ok(sections.join("\n\n"));
        }
    }

    let parsed: SummaryReport =
        serde_json::from_value(value.clone()).context("parsing summary report")?;
    Ok(render_markdown_summary(&parsed))
}

fn render_summary_markdown_from_output_with_plots(
    value: &Value,
    output_dir: &Path,
    plot_mode: plots::PlotMode,
) -> Result<String> {
    render_summary_markdown_from_output_with_plots_using_python(value, output_dir, plot_mode, None)
}

fn render_summary_markdown_from_output_with_plots_using_python(
    value: &Value,
    output_dir: &Path,
    plot_mode: plots::PlotMode,
    python_override: Option<&Path>,
) -> Result<String> {
    let plot_inputs = plots::extract_function_plot_inputs_from_output_value(value)?;
    let rendered_plots =
        plots::render_plot_artifacts(&plot_inputs, output_dir, plot_mode, python_override)?;

    if let Some(summary) = value.get("summary") {
        let parsed: SummaryReport =
            serde_json::from_value(summary.clone()).context("parsing summary report")?;
        let rendered_refs = rendered_plots.iter().collect::<Vec<_>>();
        return Ok(append_plot_links_to_markdown(
            render_markdown_summary(&parsed),
            &rendered_refs,
        ));
    }

    if let Some(targets) = value.get("targets").and_then(|v| v.as_object()) {
        let mut target_names: Vec<String> = targets.keys().cloned().collect();
        target_names.sort();

        let mut sections = Vec::new();
        for name in target_names {
            let Some(entry) = targets.get(&name) else {
                continue;
            };
            let summary_value = entry
                .get("summary")
                .cloned()
                .unwrap_or_else(|| entry.clone());
            let parsed: SummaryReport =
                serde_json::from_value(summary_value).with_context(|| {
                    format!("parsing summary report for target `{name}` in merged output")
                })?;
            let rendered_refs = rendered_plots
                .iter()
                .filter(|plot| plot.target == name)
                .collect::<Vec<_>>();
            sections.push(format!(
                "## {name}\n\n{}",
                append_plot_links_to_markdown(render_markdown_summary(&parsed), &rendered_refs)
            ));
        }
        if !sections.is_empty() {
            return Ok(sections.join("\n\n"));
        }
    }

    let parsed: SummaryReport =
        serde_json::from_value(value.clone()).context("parsing summary report")?;
    let rendered_refs = rendered_plots.iter().collect::<Vec<_>>();
    Ok(append_plot_links_to_markdown(
        render_markdown_summary(&parsed),
        &rendered_refs,
    ))
}

fn append_plot_links_to_markdown(
    mut markdown: String,
    rendered_plots: &[&plots::RenderedPlot],
) -> String {
    if rendered_plots.is_empty() {
        return markdown;
    }

    if !markdown.ends_with('\n') {
        markdown.push('\n');
    }
    markdown.push('\n');
    markdown.push_str("## Device Comparison Plots\n\n");

    for plot in rendered_plots {
        let _ = writeln!(markdown, "### {}", plot.function_label);
        let _ = writeln!(
            markdown,
            "![{}]({})",
            plot.function_label,
            plot.relative_path.display()
        );
        let _ = writeln!(markdown);
    }

    markdown
}

fn summary_markdown_output_dir(summary_path: &Path, output: Option<&Path>) -> PathBuf {
    output
        .and_then(|path| path.parent())
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| {
            summary_path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

fn upsert_github_pr_comment(pr_number: &str, marker: &str, body: &str) -> Result<()> {
    let token =
        env::var("GITHUB_TOKEN").context("provider_error: GITHUB_TOKEN is required for publish")?;
    let repository = env::var("GITHUB_REPOSITORY")
        .context("provider_error: GITHUB_REPOSITORY is required for publish")?;
    let comments_url = format!(
        "https://api.github.com/repos/{}/issues/{}/comments",
        repository, pr_number
    );
    let client = reqwest::blocking::Client::builder()
        .user_agent("mobench-report")
        .build()?;

    let comments: Vec<GitHubIssueComment> = client
        .get(&comments_url)
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .context("provider_error: listing PR comments")?
        .error_for_status()
        .context("provider_error: failed to list PR comments")?
        .json()
        .context("provider_error: failed to parse PR comments")?;

    if let Some(existing) = comments
        .into_iter()
        .find(|comment| comment.body.contains(marker))
    {
        let update_url = format!(
            "https://api.github.com/repos/{}/issues/comments/{}",
            repository, existing.id
        );
        client
            .patch(&update_url)
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .json(&json!({ "body": body }))
            .send()
            .context("provider_error: updating sticky PR comment")?
            .error_for_status()
            .context("provider_error: failed to update sticky PR comment")?;
    } else {
        client
            .post(&comments_url)
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .json(&json!({ "body": body }))
            .send()
            .context("provider_error: creating sticky PR comment")?
            .error_for_status()
            .context("provider_error: failed to create sticky PR comment")?;
    }

    Ok(())
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
            resource_usage: None,
        }],
    })
}

impl BenchmarkResourceUsage {
    fn is_empty(&self) -> bool {
        self.cpu_total_ms.is_none()
            && self.peak_memory_kb.is_none()
            && self.total_pss_kb.is_none()
            && self.private_dirty_kb.is_none()
            && self.native_heap_kb.is_none()
            && self.java_heap_kb.is_none()
    }
}

fn json_value_to_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|v| u64::try_from(v).ok()))
        .or_else(|| {
            value
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(|v| v.round() as u64)
        })
}

fn raw_peak_memory_kb(
    total_pss_kb: Option<u64>,
    private_dirty_kb: Option<u64>,
    native_heap_kb: Option<u64>,
    java_heap_kb: Option<u64>,
) -> Option<u64> {
    total_pss_kb
        .or(private_dirty_kb)
        .or_else(|| match (native_heap_kb, java_heap_kb) {
            (Some(native), Some(java)) => Some(native + java),
            (Some(native), None) => Some(native),
            (None, Some(java)) => Some(java),
            (None, None) => None,
        })
}

fn extract_benchmark_resource_usage(
    entry: &Value,
    perf_metrics: Option<&browserstack::PerformanceMetrics>,
) -> Option<BenchmarkResourceUsage> {
    let resources = entry.get("resources");
    let cpu_total_ms = resources
        .and_then(|res| res.get("elapsed_cpu_ms"))
        .and_then(json_value_to_u64);
    let total_pss_kb = resources
        .and_then(|res| res.get("total_pss_kb"))
        .and_then(json_value_to_u64);
    let private_dirty_kb = resources
        .and_then(|res| res.get("private_dirty_kb"))
        .and_then(json_value_to_u64);
    let native_heap_kb = resources
        .and_then(|res| res.get("native_heap_kb"))
        .and_then(json_value_to_u64);
    let java_heap_kb = resources
        .and_then(|res| res.get("java_heap_kb"))
        .and_then(json_value_to_u64);

    let peak_memory_kb = perf_metrics
        .and_then(|metrics| metrics.memory.as_ref())
        .map(|memory| (memory.peak_mb * 1024.0).round() as u64)
        .or_else(|| {
            raw_peak_memory_kb(total_pss_kb, private_dirty_kb, native_heap_kb, java_heap_kb)
        });

    let resource_usage = BenchmarkResourceUsage {
        cpu_total_ms,
        peak_memory_kb,
        total_pss_kb,
        private_dirty_kb,
        native_heap_kb,
        java_heap_kb,
    };

    (!resource_usage.is_empty()).then_some(resource_usage)
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

    let _ = writeln!(output, "### Benchmark Summary");
    let _ = writeln!(output);
    let _ = writeln!(output, "- Generated: {}", summary.generated_at);
    let _ = writeln!(output, "- Target: {}", summary.target.display_name());
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

fn run_android_build(
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
    };
    let builder =
        mobench_sdk::builders::AndroidBuilder::new(&layout.project_root, layout.crate_name.clone())
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
        if let Some(ndk_dir) = ndk_path.parent() {
            if ndk_dir.file_name().is_some_and(|n| n == "ndk") {
                if let Some(sdk_root) = ndk_dir.parent() {
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
    }
}

/// Load .env/.env.local from the repo root (best-effort, for commands that don't resolve a layout).
fn load_dotenv_global() {
    if let Ok(root) = repo_root() {
        let _ = dotenvy::from_path(root.join(".env"));
        let _ = dotenvy::from_path_override(root.join(".env.local"));
    }
}

fn load_dotenv_for_layout(layout: &ResolvedProjectLayout) {
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
        };

        match target {
            SdkTarget::Android => {
                println!("[1/3] Building Rust library...");
                let builder = mobench_sdk::builders::AndroidBuilder::new(
                    &layout.project_root,
                    layout.crate_name.clone(),
                )
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
                let builder = mobench_sdk::builders::IosBuilder::new(
                    &layout.project_root,
                    layout.crate_name.clone(),
                )
                .verbose(false)
                .dry_run(dry_run)
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
                let android_builder = mobench_sdk::builders::AndroidBuilder::new(
                    &layout.project_root,
                    layout.crate_name.clone(),
                )
                .verbose(false)
                .dry_run(dry_run)
                .output_dir(&effective_output_dir)
                .crate_dir(&layout.crate_dir);
                println!("[2/5] Building Android APK...");
                let android_result = android_builder.build(&build_config)?;

                println!("[3/5] Building Rust library for iOS...");
                let ios_builder = mobench_sdk::builders::IosBuilder::new(
                    &layout.project_root,
                    layout.crate_name.clone(),
                )
                .verbose(false)
                .dry_run(dry_run)
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
    };

    match target {
        SdkTarget::Android => {
            println!("\nBuilding for Android...");
            println!("  Building Rust library for Android targets...");
            let builder = mobench_sdk::builders::AndroidBuilder::new(
                &layout.project_root,
                layout.crate_name.clone(),
            )
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
            let builder = mobench_sdk::builders::IosBuilder::new(
                &layout.project_root,
                layout.crate_name.clone(),
            )
            .verbose(verbose)
            .dry_run(dry_run)
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
            let android_builder = mobench_sdk::builders::AndroidBuilder::new(
                &layout.project_root,
                layout.crate_name.clone(),
            )
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
            let ios_builder = mobench_sdk::builders::IosBuilder::new(
                &layout.project_root,
                layout.crate_name.clone(),
            )
            .verbose(verbose)
            .dry_run(dry_run)
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
            all_benchmarks.first().unwrap()
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

    let builder =
        mobench_sdk::builders::IosBuilder::new(&layout.project_root, layout.crate_name.clone())
            .verbose(true)
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

    let builder =
        mobench_sdk::builders::IosBuilder::new(&layout.project_root, layout.crate_name.clone())
            .verbose(true)
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
                    let abis = ["arm64-v8a", "armeabi-v7a", "x86_64"];
                    for abi in abis {
                        let lib_path = jni_base
                            .join(abi)
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

#[derive(Debug, Clone, Serialize)]
struct ResolvedMatrixDevice {
    name: String,
    os: String,
    os_version: String,
    identifier: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
}

/// Built-in device profiles so `devices resolve` works without a YAML file.
fn builtin_device_for_profile(
    platform: DevicePlatform,
    profile: &str,
) -> Option<ResolvedMatrixDevice> {
    let (name, os, os_version) = match (platform, profile) {
        (DevicePlatform::Ios, "low-spec") => ("iPhone 13", "ios", "15"),
        (DevicePlatform::Ios, "mid-spec") => ("iPhone 14", "ios", "16"),
        (DevicePlatform::Ios, "high-spec") => ("iPhone 16 Pro", "ios", "18"),
        (DevicePlatform::Android, "low-spec") => ("Google Pixel 6", "android", "12.0"),
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

fn cmd_devices_resolve(
    platform: DevicePlatform,
    profile: Option<String>,
    config_path: Option<&Path>,
    device_matrix_path: Option<&Path>,
    format: CheckOutputFormat,
) -> Result<()> {
    let profile_str = profile
        .as_deref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .unwrap_or("default");

    // Try loading matrix from file first; fall back to built-in profiles
    let (resolved, source) = match resolve_matrix_for_cli(config_path, device_matrix_path) {
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
            // No matrix file found — use built-in profiles
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

    match format {
        CheckOutputFormat::Text => {
            for device in &resolved {
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
        let identifier = format!("{}-{}", device.name, device.os_version);
        resolved.push(ResolvedMatrixDevice {
            name: device.name,
            os: device.os,
            os_version: device.os_version,
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
    let output = std::process::Command::new(cmd).args(args).output().ok()?;
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

/// Check prerequisites for building mobile artifacts.
///
/// This validates that all required tools and configurations are in place
/// before attempting a build.
fn cmd_check(target: SdkTarget, format: CheckOutputFormat) -> Result<()> {
    let checks = collect_prereq_checks(target);
    let issues = collect_issues(&checks);

    match format {
        CheckOutputFormat::Text => print_check_results_text(&checks, &issues),
        CheckOutputFormat::Json => print_check_results_json(&checks, &issues)?,
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

fn cmd_config_validate(config_path: &Path, format: CheckOutputFormat) -> Result<()> {
    let mut checks = Vec::new();
    let mut config: Option<BenchConfig> = None;

    match load_config(config_path) {
        Ok(cfg) => {
            checks.push(PrereqCheck {
                name: "Run config".to_string(),
                passed: true,
                detail: Some(config_path.display().to_string()),
                fix_hint: None,
            });
            config = Some(cfg);
        }
        Err(err) => {
            checks.push(PrereqCheck {
                name: "Run config".to_string(),
                passed: false,
                detail: Some(err.to_string()),
                fix_hint: Some(format!(
                    "Fix config file syntax/fields at {}",
                    config_path.display()
                )),
            });
        }
    }

    if let Some(cfg) = &config {
        match load_device_matrix(&cfg.device_matrix) {
            Ok(matrix) => {
                if let Some(tags) = cfg.device_tags.as_ref().filter(|tags| !tags.is_empty()) {
                    if let Err(err) = filter_devices_by_tags(matrix.devices, tags) {
                        checks.push(PrereqCheck {
                            name: "Device matrix".to_string(),
                            passed: false,
                            detail: Some(err.to_string()),
                            fix_hint: Some(format!(
                                "Update tags in {} or adjust device_tags in config",
                                cfg.device_matrix.display()
                            )),
                        });
                    } else {
                        checks.push(PrereqCheck {
                            name: "Device matrix".to_string(),
                            passed: true,
                            detail: Some(format!(
                                "{} (tags: {})",
                                cfg.device_matrix.display(),
                                tags.join(", ")
                            )),
                            fix_hint: None,
                        });
                    }
                } else {
                    checks.push(PrereqCheck {
                        name: "Device matrix".to_string(),
                        passed: true,
                        detail: Some(cfg.device_matrix.display().to_string()),
                        fix_hint: None,
                    });
                }
            }
            Err(err) => {
                checks.push(PrereqCheck {
                    name: "Device matrix".to_string(),
                    passed: false,
                    detail: Some(err.to_string()),
                    fix_hint: Some(format!(
                        "Fix or regenerate device matrix at {}",
                        cfg.device_matrix.display()
                    )),
                });
            }
        }

        match resolve_browserstack_credentials(Some(&cfg.browserstack)) {
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
    }

    let issues = collect_issues(&checks);
    match format {
        CheckOutputFormat::Text => print_check_results_text(&checks, &issues),
        CheckOutputFormat::Json => print_check_results_json(&checks, &issues)?,
    }

    if issues.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} issue(s) found. Fix them and rerun 'cargo mobench config validate'.",
            issues.len()
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
        CheckOutputFormat::Json => print_check_results_json(&checks, &issues)?,
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
            checks.push(check_rust_target("x86_64-apple-ios"));
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
            checks.push(check_rust_target("x86_64-apple-ios"));
        }
    }

    checks
}

fn collect_issues(checks: &[PrereqCheck]) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    for check in checks {
        if !check.passed {
            if let Some(ref fix) = check.fix_hint {
                issues.push(ValidationIssue {
                    category: issue_category_for_check(check),
                    check: check.name.clone(),
                    detail: check.detail.clone(),
                    fix_hint: fix.clone(),
                });
            }
        }
    }
    issues
}

fn issue_category_for_check(check: &PrereqCheck) -> ContractErrorCategory {
    match check.name.as_str() {
        "Run config" | "Device matrix" => ContractErrorCategory::Config,
        "BrowserStack credentials" => ContractErrorCategory::Provider,
        _ => ContractErrorCategory::Preflight,
    }
}

#[derive(Debug, Clone, Serialize)]
struct PrereqCheck {
    name: String,
    passed: bool,
    detail: Option<String>,
    fix_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ValidationIssue {
    category: ContractErrorCategory,
    check: String,
    detail: Option<String>,
    fix_hint: String,
}

fn print_check_results_text(checks: &[PrereqCheck], issues: &[ValidationIssue]) {
    for check in checks {
        let status = if check.passed { "\u{2713}" } else { "\u{2717}" };
        let detail = check.detail.as_deref().unwrap_or("");
        let category = if check.passed {
            None
        } else {
            Some(issue_category_for_check(check))
        };
        if detail.is_empty() {
            if let Some(category) = category {
                println!("{} {} [{}]", status, check.name, category_slug(category));
            } else {
                println!("{} {}", status, check.name);
            }
        } else {
            if let Some(category) = category {
                println!(
                    "{} {} [{}] ({})",
                    status,
                    check.name,
                    category_slug(category),
                    detail
                );
            } else {
                println!("{} {} ({})", status, check.name, detail);
            }
        }
    }

    if !issues.is_empty() {
        println!("\nTo fix:");
        for issue in issues {
            println!("  * [{}] {}", category_slug(issue.category), issue.fix_hint);
        }
        println!();
        let failed_count = checks.iter().filter(|c| !c.passed).count();
        println!("{} issue(s) found.", failed_count);
    } else {
        println!("\nAll prerequisites satisfied!");
    }
}

fn print_check_results_json(checks: &[PrereqCheck], issues: &[ValidationIssue]) -> Result<()> {
    let output = render_check_results_json(checks, issues);
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn render_check_results_json(checks: &[PrereqCheck], issues: &[ValidationIssue]) -> Value {
    json!({
        "checks": checks,
        "issues": issues,
        "all_passed": checks.iter().all(|c| c.passed),
        "passed_count": checks.iter().filter(|c| c.passed).count(),
        "failed_count": checks.iter().filter(|c| !c.passed).count(),
    })
}

fn category_slug(category: ContractErrorCategory) -> &'static str {
    match category {
        ContractErrorCategory::Config => "config_error",
        ContractErrorCategory::Preflight => "preflight_error",
        ContractErrorCategory::Provider => "provider_error",
        ContractErrorCategory::Build => "build_error",
        ContractErrorCategory::Benchmark => "benchmark_error",
    }
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
    use jsonschema::JSONSchema;
    use std::path::Path;
    use tempfile::TempDir;

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

[benchmarks]
default_function = "zk_mobile_bench::bench_query_proof_generation"
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
            MobileTarget::Android,
            "sample_fns::fibonacci".into(),
            5,
            1,
            vec!["pixel".into()],
            &layout,
            None,
            None,
            Vec::new(),
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
            MobileTarget::Android,
            "ignored::value".into(),
            1,
            0,
            Vec::new(),
            &layout,
            Some(config_path.as_path()),
            Some(cli_matrix_path.as_path()),
            Vec::new(),
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
        assert_eq!(
            layout.default_function.as_deref(),
            Some("zk_mobile_bench::bench_query_proof_generation")
        );
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
            MobileTarget::Ios,
            "zk_mobile_bench::bench_query_proof_generation".into(),
            1,
            0,
            vec!["iPhone 15".into()],
            &layout,
            None,
            None,
            Vec::new(),
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
            browserstack: None,
            ios_xcuitest: None,
        };
        let report = run_local_smoke(&spec).expect("local harness");
        assert!(report["samples"].is_array());
        assert_eq!(report["spec"]["name"], "noop_benchmark");
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
            MobileTarget::Ios,
            "zk_mobile_bench::bench_query_proof_generation".into(),
            1,
            0,
            vec!["iPhone 15".into()],
            &layout,
            None,
            None,
            Vec::new(),
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
    fn builtin_ios_low_spec_profile_matches_ci_deployment_target() {
        let resolved = builtin_device_for_profile(DevicePlatform::Ios, "low-spec")
            .expect("built-in low-spec iOS profile");

        assert_eq!(resolved.name, "iPhone 13");
        assert_eq!(resolved.os_version, "15");
        assert_eq!(resolved.identifier, "iPhone 13-15");
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
        assert!(markdown.contains("Median Label"));
        assert!(markdown.contains("P95 Label"));
        assert!(markdown.contains("regressed"));
        assert!(markdown.contains("improved"));
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

        let summary_schema: Value = serde_json::from_str(
            &fs::read_to_string(&summary_schema_path).expect("read summary schema"),
        )
        .expect("parse summary schema");
        let ci_schema: Value =
            serde_json::from_str(&fs::read_to_string(&ci_schema_path).expect("read ci schema"))
                .expect("parse ci schema");

        JSONSchema::options()
            .compile(&summary_schema)
            .expect("compile summary schema");
        JSONSchema::options()
            .compile(&ci_schema)
            .expect("compile ci schema");
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
                browserstack: None,
                ios_xcuitest: None,
            }),
            benchmark_results: None,
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
            device_summaries: Vec::new(),
        });

        assert!(markdown.starts_with("### Benchmark Summary\n"));
        assert!(markdown.contains("- Target: iOS"));
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

        assert!(markdown.contains("## Device Comparison Plots"));
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
            performance_metrics: None,
        };

        let summary = build_summary(&run_summary).expect("build summary");
        let value = serde_json::to_value(summary).expect("serialize summary");
        let resource_usage = &value["device_summaries"][0]["benchmarks"][0]["resource_usage"];

        assert_eq!(resource_usage["cpu_total_ms"], 482);
        assert_eq!(resource_usage["peak_memory_kb"], 654321);
        assert_eq!(resource_usage["total_pss_kb"], 654321);
        assert_eq!(resource_usage["private_dirty_kb"], 321000);
        assert_eq!(resource_usage["native_heap_kb"], 120000);
        assert_eq!(resource_usage["java_heap_kb"], 45000);
    }

    #[test]
    fn build_summary_prefers_browserstack_peak_memory_for_ci_summary() {
        let spec = RunSpec {
            target: MobileTarget::Ios,
            function: "bench_nullifier_proving_only".into(),
            iterations: 3,
            warmup: 1,
            devices: vec!["iPhone 15-17.0".into()],
            browserstack: None,
            ios_xcuitest: None,
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
        let resource_usage = &value["device_summaries"][0]["benchmarks"][0]["resource_usage"];

        assert_eq!(resource_usage["peak_memory_kb"], 249416);
        assert_eq!(resource_usage["cpu_total_ms"], Value::Null);
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
