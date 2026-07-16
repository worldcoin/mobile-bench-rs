//! Canonical report preparation and output adapters.
//!
//! This Module turns collected run evidence into one canonical summary and
//! owns the filesystem/network adapters that publish or compare it. Report
//! semantics and format rendering live in mobench-report.

use std::env;
use std::fmt::Write;
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use mobench_report::{
    BenchmarkFailureStats, BenchmarkResourceUsage, BenchmarkStats, CompareReport, DeviceSummary,
    RegressionFinding, compare_summaries as compare_summary_models, comparison_json,
    format_duration_smart, format_failure_elapsed_ms, markdown_inline_field_text,
    markdown_link_destination, render_compare_markdown, render_csv_summary, render_junit_report,
    render_markdown_summary,
};
use mobench_runtime::{
    CliV1Summary, Distribution, ResourceAccumulator, ResourceAggregate, ResourceSample,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::browserstack;
use crate::plots;
use crate::{
    MobileTarget, RemoteRun, RunSpec, RunSummary, SummaryReport, ci_env,
    infer_pr_number_from_github_ref, repo_root, write_file,
};

#[derive(Debug)]
pub(crate) struct SummaryPaths {
    pub(crate) json: PathBuf,
    pub(crate) markdown: PathBuf,
    pub(crate) csv: PathBuf,
}

pub(crate) fn resolve_summary_paths(output: Option<&Path>) -> Result<SummaryPaths> {
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

pub(crate) fn empty_summary(spec: &RunSpec) -> SummaryReport {
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

pub(crate) fn build_summary(run_summary: &RunSummary) -> Result<SummaryReport> {
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
                let sample_count = if samples.is_empty() {
                    entry
                        .get("samples")
                        .and_then(|value| value.as_u64())
                        .map(|value| value as usize)
                        .unwrap_or(0)
                } else {
                    samples.len()
                };
                let mean_ns = stats
                    .as_ref()
                    .map(|s| s.mean_ns)
                    .or_else(|| entry.get("mean_ns").and_then(|m| m.as_u64()));

                benchmarks.push(BenchmarkStats {
                    function,
                    samples: sample_count,
                    mean_ns,
                    median_ns: stats
                        .as_ref()
                        .map(|s| s.median_ns)
                        .or_else(|| entry.get("median_ns").and_then(|value| value.as_u64())),
                    p95_ns: stats
                        .as_ref()
                        .map(|s| s.p95_ns)
                        .or_else(|| entry.get("p95_ns").and_then(|value| value.as_u64())),
                    min_ns: stats
                        .as_ref()
                        .map(|s| s.min_ns)
                        .or_else(|| entry.get("min_ns").and_then(|value| value.as_u64())),
                    max_ns: stats
                        .as_ref()
                        .map(|s| s.max_ns)
                        .or_else(|| entry.get("max_ns").and_then(|value| value.as_u64())),
                    resource_usage: extract_benchmark_resource_usage(entry, perf_metrics),
                    failure: None,
                });
            }

            benchmarks.sort_by(|a, b| a.function.cmp(&b.function));
            device_summaries.push(DeviceSummary {
                device: device.clone(),
                benchmarks,
            });
        }
    }

    if let Some(failures) = &run_summary.benchmark_failures {
        for (device, entries) in failures {
            let device_index = if let Some(index) = device_summaries
                .iter()
                .position(|summary| summary.device == *device)
            {
                index
            } else {
                device_summaries.push(DeviceSummary {
                    device: device.clone(),
                    benchmarks: Vec::new(),
                });
                device_summaries.len() - 1
            };
            let device_summary = &mut device_summaries[device_index];

            for entry in entries {
                if let Some(failure) = benchmark_failure_stats(entry) {
                    device_summary.benchmarks.push(BenchmarkStats {
                        function: entry
                            .get("function_name")
                            .or_else(|| entry.get("function"))
                            .and_then(|value| value.as_str())
                            .unwrap_or(&run_summary.spec.function)
                            .to_string(),
                        samples: 0,
                        mean_ns: None,
                        median_ns: None,
                        p95_ns: None,
                        min_ns: None,
                        max_ns: None,
                        resource_usage: entry
                            .get("memory")
                            .and_then(extract_benchmark_resource_usage_from_memory),
                        failure: Some(failure),
                    });
                }
            }
            device_summary
                .benchmarks
                .sort_by(|a, b| a.function.cmp(&b.function));
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

pub(crate) fn prepare_summary_artifacts(
    summary: &RunSummary,
    paths: &SummaryPaths,
    summary_csv: bool,
    plot_mode: plots::PlotMode,
) -> Result<(Value, String, Option<String>)> {
    let summary_value = serde_json::to_value(summary).context("serializing run summary")?;
    let markdown_dir = paths.markdown.parent().unwrap_or_else(|| Path::new("."));
    let markdown =
        render_summary_markdown_from_output_with_plots(&summary_value, markdown_dir, plot_mode)?;
    let csv = summary_csv.then(|| render_csv_summary(&summary.summary));
    Ok((summary_value, markdown, csv))
}

pub(crate) const EXIT_REGRESSION: i32 = 2;

pub(crate) fn append_github_step_summary_from_path(path: &Path) -> Result<()> {
    let Ok(summary_path) = env::var("GITHUB_STEP_SUMMARY") else {
        return Ok(());
    };
    let contents =
        fs::read_to_string(path).with_context(|| format!("reading summary markdown {:?}", path))?;
    append_github_step_summary(&contents, &summary_path)
}

pub(crate) fn append_github_step_summary(contents: &str, summary_path: &str) -> Result<()> {
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

pub(crate) fn write_junit_report(
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

/// Print a final summary with all artifact correlation information (C3).
#[allow(dead_code)]
pub(crate) fn print_run_completion_summary(
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
                if let Some(failure) = &bench.failure {
                    println!(
                        "    {} - failed: {}, elapsed: {}",
                        bench.function,
                        failure.kind,
                        format_failure_elapsed_ms(Some(failure))
                    );
                    continue;
                }
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

pub(crate) fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("creating directory {:?}", parent))?;
    }
    Ok(())
}

pub(crate) fn compare_summaries(baseline: &Path, candidate: &Path) -> Result<CompareReport> {
    let baseline_summary = load_run_summary(baseline)?;
    let candidate_summary = load_run_summary(candidate)?;

    Ok(compare_run_summaries(
        baseline,
        candidate,
        &baseline_summary,
        &candidate_summary,
    ))
}

pub(crate) fn compare_run_summaries(
    baseline: &Path,
    candidate: &Path,
    baseline_summary: &RunSummary,
    candidate_summary: &RunSummary,
) -> CompareReport {
    compare_summary_models(
        baseline,
        candidate,
        &baseline_summary.summary,
        &candidate_summary.summary,
    )
}

pub(crate) fn load_run_summary(path: &Path) -> Result<RunSummary> {
    let contents = fs::read_to_string(path).with_context(|| format!("reading {:?}", path))?;
    serde_json::from_str(&contents).with_context(|| format!("parsing summary {:?}", path))
}

pub(crate) fn resolve_baseline_source(source: &str) -> Result<PathBuf> {
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

pub(crate) fn paths_point_to_same_file(lhs: &Path, rhs: &Path) -> Result<bool> {
    Ok(normalized_path(lhs)? == normalized_path(rhs)?)
}

pub(crate) fn snapshot_baseline_for_compare(path: &Path) -> Result<PathBuf> {
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

pub(crate) fn inject_compare_into_summary_value(
    summary_value: &mut Value,
    report: &CompareReport,
    threshold_pct: f64,
    baseline_source: Option<&str>,
) {
    if let Some(obj) = summary_value.as_object_mut() {
        obj.insert(
            "comparison".to_string(),
            comparison_json(report, threshold_pct, baseline_source),
        );
    }
}

pub(crate) fn write_compare_report(report: &CompareReport, output: Option<&Path>) -> Result<()> {
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

#[derive(Debug, Deserialize)]
struct GitHubIssueComment {
    id: u64,
    body: String,
}

pub(crate) fn cmd_report_summarize(
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

pub(crate) fn cmd_report_github(
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

pub(crate) fn render_summary_markdown_from_output(value: &Value) -> Result<String> {
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
            sections.push(format!(
                "## {}\n\n{}",
                markdown_inline_field_text(&name),
                render_markdown_summary(&parsed)
            ));
        }
        if !sections.is_empty() {
            return Ok(sections.join("\n\n"));
        }
    }

    let parsed: SummaryReport =
        serde_json::from_value(value.clone()).context("parsing summary report")?;
    Ok(render_markdown_summary(&parsed))
}

pub(crate) fn render_summary_markdown_from_output_with_plots(
    value: &Value,
    output_dir: &Path,
    plot_mode: plots::PlotMode,
) -> Result<String> {
    render_summary_markdown_from_output_with_plots_using_python(value, output_dir, plot_mode, None)
}

pub(crate) fn render_summary_markdown_from_output_with_plots_using_python(
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
                "## {}\n\n{}",
                markdown_inline_field_text(&name),
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

pub(crate) fn append_plot_links_to_markdown(
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
    markdown.push_str("### Device Comparison Plots\n\n");

    for plot in rendered_plots {
        let label = markdown_inline_field_text(&plot.function_label);
        let relative_path = plot.relative_path.to_string_lossy().replace('\\', "/");
        let destination = markdown_link_destination(&relative_path);
        let _ = writeln!(markdown, "### {label}");
        let _ = writeln!(markdown, "![{label}]({destination})",);
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
    // Validate inputs to prevent URL path injection
    if pr_number.is_empty() || !pr_number.chars().all(|c| c.is_ascii_digit()) {
        bail!("PR number must be numeric, got: {}", pr_number);
    }
    let token =
        env::var("GITHUB_TOKEN").context("provider_error: GITHUB_TOKEN is required for publish")?;
    let repository = env::var("GITHUB_REPOSITORY")
        .context("provider_error: GITHUB_REPOSITORY is required for publish")?;
    if repository.matches('/').count() != 1
        || repository
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && !matches!(c, '/' | '-' | '_' | '.'))
    {
        bail!(
            "GITHUB_REPOSITORY must be owner/repo format, got: {}",
            repository
        );
    }
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
            resource_usage: extract_benchmark_resource_usage(&run_summary.local_report, None),
            failure: None,
        }],
    })
}

type SampleStats = CliV1Summary;

pub(crate) fn compute_sample_stats(samples: &[u64]) -> Option<SampleStats> {
    Distribution::from_slice(samples).cli_v1_summary()
}

pub(crate) fn extract_samples(value: &Value) -> Vec<u64> {
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

pub(crate) fn json_value_to_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| {
            value
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .map(|value| value.round() as u64)
        })
}

pub(crate) fn json_value_to_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .and_then(|value| u32::try_from(value).ok())
}

fn extract_sample_resources(value: &Value) -> ResourceAggregate {
    let mut resources = ResourceAccumulator::new();
    if let Some(samples) = value.get("samples").and_then(Value::as_array) {
        for sample in samples {
            resources.record(ResourceSample {
                cpu_time_ms: sample.get("cpu_time_ms").and_then(json_value_to_u64),
                peak_memory_growth_kb: sample.get("peak_memory_kb").and_then(json_value_to_u64),
                process_peak_memory_kb: sample
                    .get("process_peak_memory_kb")
                    .and_then(json_value_to_u64),
            });
        }
    }
    resources.finish()
}

pub(crate) fn extract_benchmark_resource_usage(
    entry: &Value,
    _perf_metrics: Option<&browserstack::PerformanceMetrics>,
) -> Option<BenchmarkResourceUsage> {
    let resources = entry
        .get("resource_usage")
        .or_else(|| entry.get("resources"))
        .or(Some(entry));
    let sample_resources = extract_sample_resources(entry);

    let cpu_total_ms = resources
        .and_then(|res| res.get("cpu_total_ms"))
        .and_then(json_value_to_u64)
        .or_else(|| {
            resources
                .and_then(|res| res.get("elapsed_cpu_ms"))
                .and_then(json_value_to_u64)
        })
        .or(sample_resources.cpu_total_ms);
    let cpu_median_ms = resources
        .and_then(|res| res.get("cpu_median_ms"))
        .and_then(json_value_to_u64)
        .or(sample_resources.cpu_median_ms);
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
    let explicit_peak_memory_growth_kb = resources
        .and_then(|res| res.get("peak_memory_growth_kb"))
        .and_then(json_value_to_u64);
    let legacy_peak_memory_kb = resources
        .and_then(|res| res.get("peak_memory_kb"))
        .and_then(json_value_to_u64);
    let peak_memory_growth_kb = explicit_peak_memory_growth_kb
        .or(legacy_peak_memory_kb)
        .or(sample_resources.peak_memory_growth_kb);
    let peak_memory_kb = peak_memory_growth_kb;
    let process_peak_memory_kb = resources
        .and_then(|res| res.get("process_peak_memory_kb"))
        .and_then(json_value_to_u64)
        .or(sample_resources.process_peak_memory_kb);

    let resource_usage = BenchmarkResourceUsage {
        cpu_total_ms,
        cpu_median_ms,
        peak_memory_kb,
        peak_memory_growth_kb,
        process_peak_memory_kb,
        total_pss_kb,
        private_dirty_kb,
        native_heap_kb,
        java_heap_kb,
    };

    (!resource_usage.is_empty()).then_some(resource_usage)
}

fn extract_benchmark_resource_usage_from_memory(memory: &Value) -> Option<BenchmarkResourceUsage> {
    let resource_usage = BenchmarkResourceUsage {
        cpu_total_ms: None,
        cpu_median_ms: None,
        peak_memory_kb: None,
        peak_memory_growth_kb: None,
        process_peak_memory_kb: memory.get("process_pss_kb").and_then(json_value_to_u64),
        total_pss_kb: memory.get("total_pss_kb").and_then(json_value_to_u64),
        private_dirty_kb: memory.get("private_dirty_kb").and_then(json_value_to_u64),
        native_heap_kb: memory.get("native_heap_kb").and_then(json_value_to_u64),
        java_heap_kb: memory.get("java_heap_kb").and_then(json_value_to_u64),
    };

    (!resource_usage.is_empty()).then_some(resource_usage)
}

fn benchmark_failure_stats(entry: &Value) -> Option<BenchmarkFailureStats> {
    let kind = entry.get("kind").and_then(|value| value.as_str())?;
    let message = entry
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or("no message")
        .to_string();
    let exit_reason = entry
        .get("android_exit_info")
        .and_then(|info| info.get("reason"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);

    Some(BenchmarkFailureStats {
        kind: kind.to_string(),
        message,
        elapsed_ms: entry.get("elapsed_ms").and_then(json_value_to_u64),
        exit_reason,
    })
}
