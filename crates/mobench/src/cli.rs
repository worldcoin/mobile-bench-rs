use clap::{Args, Parser, Subcommand, ValueEnum};
use mobench_runtime::MAX_BENCHMARK_COUNT;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::{plots, profile};

fn parse_iterations(value: &str) -> Result<u32, String> {
    let count = value
        .parse::<u32>()
        .map_err(|_| "iterations must be an unsigned 32-bit integer".to_string())?;
    if count == 0 {
        return Err("iterations must be greater than zero".to_string());
    }
    if count > MAX_BENCHMARK_COUNT {
        return Err(format!("iterations must not exceed {MAX_BENCHMARK_COUNT}"));
    }
    Ok(count)
}

fn parse_warmup(value: &str) -> Result<u32, String> {
    let count = value
        .parse::<u32>()
        .map_err(|_| "warmup must be an unsigned 32-bit integer".to_string())?;
    if count > MAX_BENCHMARK_COUNT {
        return Err(format!("warmup must not exceed {MAX_BENCHMARK_COUNT}"));
    }
    Ok(count)
}

/// CLI orchestrator for building, packaging, and executing Rust benchmarks on mobile.
#[derive(Parser, Debug)]
#[command(name = "mobench", author, version, about = "Mobile Rust benchmarking orchestrator", long_about = None)]
pub(crate) struct Cli {
    /// Print what would be done without actually doing it
    #[arg(long, global = true)]
    pub(crate) dry_run: bool,

    /// Print verbose output including all commands
    #[arg(long, short = 'v', global = true)]
    pub(crate) verbose: bool,

    /// Assume yes to prompts and allow overwriting files
    #[arg(long, global = true)]
    pub(crate) yes: bool,

    /// Disable interactive prompts (fail instead)
    #[arg(long, global = true)]
    pub(crate) non_interactive: bool,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Run benchmarks locally or on BrowserStack devices.
    ///
    /// This is a single-command flow that:
    /// 1. Builds Rust libraries for the target platform
    /// 2. Packages mobile apps (APK/IPA) automatically
    /// 3. Uploads to BrowserStack when devices are requested
    /// 4. Schedules the benchmark run when using BrowserStack
    /// 5. Fetches results when the provider returns them
    ///
    /// For iOS, IPA and XCUITest packages are created automatically unless
    /// you provide --ios-app and --ios-test-suite to override.
    Run {
        #[arg(long, value_enum)]
        target: Option<MobileTarget>,
        #[arg(long, help = "Fully-qualified Rust function to benchmark")]
        function: Option<String>,
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
        #[arg(long, value_parser = parse_iterations)]
        iterations: Option<u32>,
        #[arg(long, value_parser = parse_warmup)]
        warmup: Option<u32>,
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
        #[arg(
            long,
            hide = true,
            help = "Deprecated compatibility flag for generated XCUITest harness timeout"
        )]
        ios_completion_timeout_secs: Option<u64>,
        #[arg(
            long,
            help = "iOS deployment target for generated app and XCUITest targets"
        )]
        ios_deployment_target: Option<String>,
        #[arg(
            long,
            value_enum,
            help = "iOS runner template (swiftui or uikit-legacy)"
        )]
        ios_runner: Option<IosRunnerArg>,
        #[arg(
            long,
            help = "Android benchmark watchdog timeout in seconds for the generated harness"
        )]
        android_benchmark_timeout_secs: Option<u64>,
        #[arg(
            long,
            help = "Android benchmark heartbeat interval in seconds for the generated harness"
        )]
        android_heartbeat_interval_secs: Option<u64>,
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
    /// Initialize a new benchmark project with the SDK templates.
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
    /// Build mobile artifacts from the resolved benchmark crate.
    Build {
        #[arg(long, value_enum)]
        target: SdkTarget,
        #[arg(long, help = "Build in release mode")]
        release: bool,
        #[arg(
            long,
            hide = true,
            help = "Deprecated compatibility flag for generated XCUITest harness timeout"
        )]
        ios_completion_timeout_secs: Option<u64>,
        #[arg(
            long,
            help = "iOS deployment target for generated app and XCUITest targets"
        )]
        ios_deployment_target: Option<String>,
        #[arg(
            long,
            value_enum,
            help = "iOS runner template (swiftui or uikit-legacy)"
        )]
        ios_runner: Option<IosRunnerArg>,
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
    /// List all discovered benchmark functions.
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
#[allow(clippy::large_enum_variant)]
pub(crate) enum CiCommand {
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
    /// Merge one-sample CI summaries into a normal CI output set.
    MergeSplitRuns(CiMergeSplitRunsArgs),
    /// Summarize benchmark results with device metrics.
    Summarize(CiSummarizeArgs),
    /// Create a GitHub Check Run with benchmark results.
    CheckRun(CiCheckRunArgs),
}

#[derive(Subcommand, Debug)]
pub(crate) enum DevicesCommand {
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
pub(crate) enum ConfigCommand {
    /// Validate bench-config.toml and referenced matrix/settings.
    Validate {
        #[arg(long, default_value = "bench-config.toml")]
        config: PathBuf,
        #[arg(long, value_enum, default_value_t = CheckOutputFormat::Text)]
        format: CheckOutputFormat,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum PlotFixture {
    Basic,
    Ffi,
}

#[derive(Subcommand, Debug)]
pub(crate) enum FixtureCommand {
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
    /// Render and verify checked-in plot fixtures.
    VerifyPlots {
        #[arg(value_enum)]
        fixture: PlotFixture,
        #[arg(
            long,
            help = "Output directory for rendered fixture artifacts (defaults under target/mobench/plot-fixtures)"
        )]
        output_dir: Option<PathBuf>,
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
pub(crate) enum ReportCommand {
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
pub(crate) enum ProfileCommand {
    #[command(
        about = "Plan or execute a native profiling session; local android-native and ios-instruments now attempt real native capture"
    )]
    Run(profile::ProfileRunArgs),
    /// Generate a differential flamegraph bundle from two normalized profile manifests.
    Diff(profile::ProfileDiffArgs),
    /// Render markdown or JSON from a normalized profile manifest.
    Summarize(profile::ProfileSummarizeArgs),
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CiRunArgs {
    #[arg(long, value_enum)]
    pub(crate) target: CiTarget,
    #[arg(
        long,
        help = "Path to the benchmark crate directory containing Cargo.toml"
    )]
    pub(crate) crate_path: Option<PathBuf>,
    #[arg(
        long,
        help = "Fully-qualified Rust function to benchmark (single function)"
    )]
    pub(crate) function: Option<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Multiple benchmark functions (comma-separated or JSON array). Runs each in sequence."
    )]
    pub(crate) functions: Vec<String>,
    #[arg(long, default_value_t = 100, value_parser = parse_iterations)]
    pub(crate) iterations: u32,
    #[arg(long, default_value_t = 10, value_parser = parse_warmup)]
    pub(crate) warmup: u32,
    #[arg(long, help = "Device identifiers or labels (BrowserStack devices)")]
    pub(crate) devices: Vec<String>,
    #[arg(long, help = "Device matrix YAML file to load device names from")]
    pub(crate) device_matrix: Option<PathBuf>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Device tags to select from the device matrix (comma-separated or repeatable)"
    )]
    pub(crate) device_tags: Vec<String>,
    #[arg(long, help = "Optional path to config file")]
    pub(crate) config: Option<PathBuf>,
    #[arg(long, help = "Baseline summary source (path|url|artifact:<path>)")]
    pub(crate) baseline: Option<String>,
    #[arg(
        long,
        default_value_t = 5.0,
        help = "Regression threshold percentage when comparing to baseline"
    )]
    pub(crate) regression_threshold_pct: f64,
    #[arg(long, help = "Write JUnit XML report to the given path")]
    pub(crate) junit: Option<PathBuf>,
    #[arg(long, help = "Skip mobile builds and only run the host harness")]
    pub(crate) local_only: bool,
    #[arg(
        long,
        help = "Build in release mode (recommended for BrowserStack to reduce APK size and upload time)"
    )]
    pub(crate) release: bool,
    #[arg(
        long,
        help = "Path to iOS app bundle (.ipa or zipped .app) for BrowserStack XCUITest"
    )]
    pub(crate) ios_app: Option<PathBuf>,
    #[arg(long, help = "Path to iOS XCUITest test suite package (.zip or .ipa)")]
    pub(crate) ios_test_suite: Option<PathBuf>,
    #[arg(
        long,
        hide = true,
        help = "Deprecated compatibility flag for generated XCUITest harness timeout"
    )]
    pub(crate) ios_completion_timeout_secs: Option<u64>,
    #[arg(
        long,
        help = "iOS deployment target for generated app and XCUITest targets"
    )]
    pub(crate) ios_deployment_target: Option<String>,
    #[arg(
        long,
        value_enum,
        help = "iOS runner template (swiftui or uikit-legacy)"
    )]
    pub(crate) ios_runner: Option<IosRunnerArg>,
    #[arg(
        long,
        help = "Android benchmark watchdog timeout in seconds for the generated harness"
    )]
    pub(crate) android_benchmark_timeout_secs: Option<u64>,
    #[arg(
        long,
        help = "Android benchmark heartbeat interval in seconds for the generated harness"
    )]
    pub(crate) android_heartbeat_interval_secs: Option<u64>,
    #[arg(long, help = "Fetch BrowserStack artifacts after the run completes")]
    pub(crate) fetch: bool,
    #[arg(long, default_value = "target/browserstack")]
    pub(crate) fetch_output_dir: PathBuf,
    #[arg(long, default_value_t = 5)]
    pub(crate) fetch_poll_interval_secs: u64,
    #[arg(long, default_value_t = 300)]
    pub(crate) fetch_timeout_secs: u64,
    #[arg(long, help = "Show simplified step-by-step progress output")]
    pub(crate) progress: bool,
    #[arg(
        long,
        default_value = "target/mobench/ci",
        help = "Output directory for CI contract files"
    )]
    pub(crate) output_dir: PathBuf,
    #[arg(long, help = "Metadata: user or actor that requested the run")]
    pub(crate) requested_by: Option<String>,
    #[arg(long, help = "Metadata: pull request number")]
    pub(crate) pr_number: Option<String>,
    #[arg(long, help = "Metadata: original command requested by the caller")]
    pub(crate) request_command: Option<String>,
    #[arg(long, help = "Metadata: git ref/sha for this mobench invocation")]
    pub(crate) mobench_ref: Option<String>,
    #[arg(long, value_enum, default_value_t = plots::PlotMode::Auto)]
    pub(crate) plots: plots::PlotMode,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CiSummarizeArgs {
    /// BrowserStack build ID to enrich results with device metrics (requires --results-dir).
    #[arg(long)]
    pub(crate) build_id: Option<String>,

    /// Directory containing summary.json/CSV results (offline mode).
    #[arg(long)]
    pub(crate) results_dir: Option<PathBuf>,

    /// Output format: table (terminal), markdown, or json.
    #[arg(long, value_enum, default_value_t = SummarizeFormat::Table)]
    pub(crate) output_format: SummarizeFormat,

    /// Write output to file in addition to stdout.
    #[arg(long)]
    pub(crate) output_file: Option<PathBuf>,

    /// Platform filter (show only one platform).
    #[arg(long, value_enum)]
    pub(crate) platform: Option<MobileTarget>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CiMergeSplitRunsArgs {
    /// Directory containing sample-*/summary.json one-sample CI outputs.
    #[arg(long)]
    pub(crate) samples_dir: PathBuf,

    /// Directory to write merged summary.json, summary.md, and results.csv.
    #[arg(long)]
    pub(crate) output_dir: PathBuf,

    /// Fully-qualified benchmark function all samples must contain.
    #[arg(long)]
    pub(crate) function: String,

    /// Device label all samples must match.
    #[arg(long)]
    pub(crate) device: String,

    /// Expected measured sample count.
    #[arg(long, value_parser = parse_iterations)]
    pub(crate) iterations: u32,

    /// Warmup count reported in merged CI summaries.
    #[arg(long, default_value_t = 0, value_parser = parse_warmup)]
    pub(crate) warmup: u32,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CiCheckRunArgs {
    /// Path to summary JSON with benchmark results.
    #[arg(long, required_unless_present = "results_dir")]
    pub(crate) results: Option<PathBuf>,

    /// Directory containing summary JSON files (processes all).
    #[arg(long, required_unless_present = "results")]
    pub(crate) results_dir: Option<PathBuf>,

    /// GitHub repository (owner/repo format).
    #[arg(long)]
    pub(crate) repo: String,

    /// Git commit SHA to annotate.
    #[arg(long)]
    pub(crate) sha: String,

    /// GitHub App token (from GITHUB_TOKEN env var or actions/create-github-app-token).
    #[arg(long, env = "GITHUB_TOKEN", hide = true)]
    pub(crate) token: String,

    /// Check Run name displayed in the PR.
    #[arg(long, default_value = "Mobench")]
    pub(crate) name: String,

    /// Optional baseline JSON for regression detection.
    #[arg(long)]
    pub(crate) baseline: Option<PathBuf>,

    /// Regression threshold percentage.
    #[arg(long, default_value_t = 5.0)]
    pub(crate) regression_threshold_pct: f64,

    /// File path used in Check Run annotations (relative to repo root).
    #[arg(long, default_value = "src/lib.rs")]
    pub(crate) annotation_path: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SummarizeFormat {
    Table,
    Markdown,
    Json,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CiTarget {
    Android,
    Ios,
    Both,
}

impl CiTarget {
    pub(crate) fn targets(self) -> &'static [MobileTarget] {
        match self {
            CiTarget::Android => &[MobileTarget::Android],
            CiTarget::Ios => &[MobileTarget::Ios],
            CiTarget::Both => &[MobileTarget::Android, MobileTarget::Ios],
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub(crate) enum DevicePlatform {
    Android,
    Ios,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub(crate) enum SummaryFormat {
    Text,
    Json,
    Csv,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub(crate) enum CheckOutputFormat {
    Text,
    Json,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContractErrorCategory {
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
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Ios => "ios",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Android => "Android",
            Self::Ios => "iOS",
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub(crate) enum SdkTarget {
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
pub(crate) enum IosSigningMethodArg {
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

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[clap(rename_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum IosRunnerArg {
    Swiftui,
    UikitLegacy,
}

impl From<IosRunnerArg> for mobench_sdk::codegen::IosRunner {
    fn from(arg: IosRunnerArg) -> Self {
        match arg {
            IosRunnerArg::Swiftui => mobench_sdk::codegen::IosRunner::Swiftui,
            IosRunnerArg::UikitLegacy => mobench_sdk::codegen::IosRunner::UikitLegacy,
        }
    }
}
