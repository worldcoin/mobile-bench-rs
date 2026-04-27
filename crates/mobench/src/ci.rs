use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::browserstack::{BrowserStackAuth, BrowserStackClient};
use crate::cli::{CiCheckRunArgs, CiRunArgs, CiSummarizeArgs, SummarizeFormat};
use crate::{
    BenchmarkStats, DeviceSummary, EXIT_REGRESSION, MobileTarget, SummaryReport, ensure_can_write,
    github, plots, render_csv_summary, render_summary_markdown_from_output_with_plots,
    resolve_browserstack_credentials, summarize, write_file,
};

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
    /// Deprecated compatibility timeout for generated iOS benchmark harnesses.
    pub ios_completion_timeout_secs: Option<u64>,
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

#[cfg(feature = "bench-support")]
#[doc(hidden)]
pub mod bench_support {
    use super::*;
    use crate::{BenchConfig, DeviceMatrix, profile, render_markdown_summary};

    pub fn parse_run_config(input: &str) -> Result<()> {
        let _: BenchConfig = toml::from_str(input)?;
        Ok(())
    }

    pub fn parse_device_matrix(input: &str) -> Result<()> {
        let _: DeviceMatrix = serde_yaml::from_str(input)?;
        Ok(())
    }

    pub fn render_summary_markdown_from_json(input: &str) -> Result<String> {
        let summary = summary_report_from_json(input)?;
        Ok(render_markdown_summary(&summary))
    }

    pub fn render_summary_csv_from_json(input: &str) -> Result<String> {
        let summary = summary_report_from_json(input)?;
        Ok(render_csv_summary(&summary))
    }

    pub fn render_profile_markdown_from_json(input: &str) -> Result<String> {
        let manifest: profile::ProfileManifest = serde_json::from_str(input)?;
        Ok(profile::render_profile_markdown(&manifest))
    }

    pub fn extract_browserstack_results_from_logs(logs: &str) -> Result<usize> {
        let client = BrowserStackClient::new(
            BrowserStackAuth {
                username: "benchmark".to_string(),
                access_key: "benchmark".to_string(),
            },
            None,
        )?;
        Ok(client.extract_benchmark_results(logs)?.len())
    }

    fn summary_report_from_json(input: &str) -> Result<SummaryReport> {
        let value: Value = serde_json::from_str(input)?;
        let summary = value.get("summary").cloned().unwrap_or(value);
        Ok(serde_json::from_value(summary)?)
    }
}

/// Executes a [`RunRequest`] by invoking a `mobench` command and normalizing outputs.
///
/// This function always writes/normalizes CI output file names in `request.output_dir`:
/// - `summary.json`
/// - `summary.md`
/// - `results.csv`
///
/// Returns [`RunResult`] containing file paths and process exit semantics.
pub fn run_request(request: &RunRequest) -> Result<RunResult> {
    run_request_with_mode(request, false)
}

fn run_request_with_mode(request: &RunRequest, dry_run: bool) -> Result<RunResult> {
    if !dry_run {
        fs::create_dir_all(&request.output_dir)
            .with_context(|| format!("creating output dir {}", request.output_dir.display()))?;
    }

    let summary_json = request.output_dir.join("summary.json");
    let summary_md = request.output_dir.join("summary.md");
    let summary_csv = request.output_dir.join("summary.csv");
    let results_csv = request.output_dir.join("results.csv");

    let mut cmd = mobench_command()?;
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
    if let Some(timeout_secs) = request.ios_completion_timeout_secs {
        cmd.arg("--ios-completion-timeout-secs")
            .arg(timeout_secs.to_string());
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
    if dry_run {
        cmd.arg("--dry-run");
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
    if dry_run {
        return Ok(RunResult {
            target: request.target,
            report: Report {
                summary_json,
                summary_md,
                results_csv,
            },
            exit_code,
            regression_detected: exit_code == EXIT_REGRESSION,
        });
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

fn mobench_command() -> Result<std::process::Command> {
    if let Some(path) = env::var_os("MOBENCH_BIN")
        .or_else(|| env::var_os("CARGO_BIN_EXE_mobench"))
        .or_else(|| env::var_os("CARGO_BIN_EXE_cargo-mobench"))
    {
        return Ok(std::process::Command::new(path));
    }

    let current_exe = env::current_exe().context("resolving current executable")?;
    let current_name = current_exe
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if matches!(current_name, "mobench" | "cargo-mobench") {
        return Ok(std::process::Command::new(current_exe));
    }

    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("mobench");
    Ok(cmd)
}

fn resolve_ci_functions(args: &CiRunArgs) -> Result<Vec<String>> {
    let mut funcs = args.functions.clone();

    // --function (singular) is sugar for a single-element list
    if let Some(ref f) = args.function
        && !funcs.contains(f)
    {
        funcs.insert(0, f.clone());
    }

    // Support JSON array passed as a single element: '["a","b"]'
    if funcs.len() == 1 {
        let trimmed = funcs[0].trim();
        if trimmed.starts_with('[')
            && let Ok(parsed) = serde_json::from_str::<Vec<String>>(trimmed)
        {
            return Ok(parsed);
        }
    }

    if funcs.is_empty() {
        bail!("At least one benchmark function is required. Use --function or --functions.");
    }
    Ok(funcs)
}

pub(crate) fn ci_function_slug(function: &str) -> String {
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

pub(crate) fn find_baseline_benchmark<'a>(
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

pub(crate) fn summary_report_from_value(value: &Value) -> Result<SummaryReport> {
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

pub(crate) fn merge_ci_target_runs(
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

pub(crate) fn root_summary_from_merged_targets(targets: &BTreeMap<String, Value>) -> Option<Value> {
    if targets.len() != 1 {
        return None;
    }

    targets
        .values()
        .next()
        .and_then(|entry| entry.get("summary").cloned())
}

pub(crate) fn cmd_ci_run(args: CiRunArgs, dry_run: bool) -> Result<()> {
    let all_functions = resolve_ci_functions(&args)?;

    if !dry_run {
        fs::create_dir_all(&args.output_dir)
            .with_context(|| format!("creating ci output dir {}", args.output_dir.display()))?;
    }
    let metadata = ci_metadata_from_args(&args);
    let targets = args.target.targets();

    if targets.len() == 1 && all_functions.len() == 1 {
        // Fast path: single target, single function — original behavior
        let target = targets[0];
        let mut single_args = args.clone();
        single_args.function = Some(all_functions[0].clone());
        let exit_code =
            cmd_ci_run_single(&single_args, target, &args.output_dir, &metadata, dry_run)?;
        if dry_run {
            println!("CI dry run complete; no outputs written.");
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            return Ok(());
        }

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
            if !dry_run {
                fs::create_dir_all(&target_dir).with_context(|| {
                    format!("creating target output dir {}", target_dir.display())
                })?;
            }
            let mut func_args = args.clone();
            func_args.function = Some(func.clone());
            let exit_code =
                cmd_ci_run_single(&func_args, target_value, &target_dir, &metadata, dry_run)?;
            if exit_code == EXIT_REGRESSION {
                regression_detected = true;
            } else if exit_code != 0 {
                std::process::exit(exit_code);
            }
            if dry_run {
                continue;
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

    if dry_run {
        println!("CI dry run complete; no outputs written.");
        return Ok(());
    }

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

pub(crate) fn cmd_ci_summarize(args: CiSummarizeArgs) -> Result<()> {
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

pub(crate) fn cmd_ci_check_run(args: CiCheckRunArgs) -> Result<()> {
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

                if let Some(base) = baseline_bench
                    && base.timing.avg_ms > 0.0
                {
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
    dry_run: bool,
) -> Result<i32> {
    let default_baseline_path = previous_baseline_path(output_dir);
    let baseline_source = args.baseline.clone().or_else(|| {
        if default_baseline_path.exists() {
            Some(default_baseline_path.display().to_string())
        } else {
            None
        }
    });

    let request = RunRequest {
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
        ios_completion_timeout_secs: args.ios_completion_timeout_secs,
        fetch: args.fetch,
        fetch_output_dir: args.fetch_output_dir.clone(),
        fetch_poll_interval_secs: args.fetch_poll_interval_secs,
        fetch_timeout_secs: args.fetch_timeout_secs,
        progress: args.progress,
        output_dir: output_dir.to_path_buf(),
        plots: args.plots,
    };
    let result = run_request_with_mode(&request, dry_run)?;

    if dry_run {
        return Ok(result.exit_code);
    }

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

pub(crate) fn ci_env(keys: &[&str]) -> Option<String> {
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

pub(crate) fn infer_pr_number_from_github_ref() -> Option<String> {
    let github_ref = env::var("GITHUB_REF").ok()?;
    parse_pr_number_from_ref(&github_ref)
}

pub(crate) fn parse_pr_number_from_ref(github_ref: &str) -> Option<String> {
    let parts: Vec<&str> = github_ref.split('/').collect();
    if parts.len() >= 4 && parts[0] == "refs" && parts[1] == "pull" {
        let pr = parts[2].trim();
        if !pr.is_empty() {
            return Some(pr.to_string());
        }
    }
    None
}

pub(crate) fn cmd_ci_init(workflow_path: &Path, action_dir: &Path, overwrite: bool) -> Result<()> {
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

pub(crate) fn fetch_browserstack_artifacts(
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
