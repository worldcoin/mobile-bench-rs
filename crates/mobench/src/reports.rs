use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fmt::Write;
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::browserstack;
use crate::cli::SummaryFormat;
use crate::{
    BenchmarkResourceUsage, BenchmarkStats, DeviceSummary, RunSpec, RunSummary, SummaryReport,
    ci_env, infer_pr_number_from_github_ref, plots, write_file,
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

pub(crate) fn write_summary(
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

pub(crate) fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("creating directory {:?}", parent))?;
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
    markdown.push_str("### Device Comparison Plots\n\n");

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
        }],
    })
}

impl BenchmarkResourceUsage {
    fn peak_memory_growth_or_legacy_kb(&self) -> Option<u64> {
        self.peak_memory_growth_kb.or(self.peak_memory_kb)
    }

    fn is_empty(&self) -> bool {
        self.cpu_total_ms.is_none()
            && self.cpu_median_ms.is_none()
            && self.peak_memory_kb.is_none()
            && self.peak_memory_growth_kb.is_none()
            && self.process_peak_memory_kb.is_none()
            && self.total_pss_kb.is_none()
            && self.private_dirty_kb.is_none()
            && self.native_heap_kb.is_none()
            && self.java_heap_kb.is_none()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SampleStats {
    pub(crate) mean_ns: u64,
    pub(crate) median_ns: u64,
    pub(crate) p95_ns: u64,
    pub(crate) min_ns: u64,
    pub(crate) max_ns: u64,
}

pub(crate) fn compute_sample_stats(samples: &[u64]) -> Option<SampleStats> {
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

fn extract_sample_metric_u64(value: &Value, key: &str) -> Vec<u64> {
    value
        .get("samples")
        .and_then(|samples| samples.as_array())
        .map(|samples| {
            samples
                .iter()
                .filter_map(|sample| sample.get(key))
                .filter_map(json_value_to_u64)
                .collect()
        })
        .unwrap_or_default()
}

fn json_value_to_u64(value: &Value) -> Option<u64> {
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

fn median_u64(values: &[u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let len = sorted.len();
    Some(if len % 2 == 0 {
        let lower = u128::from(sorted[(len / 2) - 1]);
        let upper = u128::from(sorted[len / 2]);
        ((lower + upper) / 2) as u64
    } else {
        sorted[len / 2]
    })
}

pub(crate) fn extract_benchmark_resource_usage(
    entry: &Value,
    _perf_metrics: Option<&browserstack::PerformanceMetrics>,
) -> Option<BenchmarkResourceUsage> {
    let resources = entry
        .get("resource_usage")
        .or_else(|| entry.get("resources"))
        .or(Some(entry));
    let sample_cpu_ms = extract_sample_metric_u64(entry, "cpu_time_ms");
    let sample_peak_memory_kb = extract_sample_metric_u64(entry, "peak_memory_kb");
    let sample_process_peak_memory_kb = extract_sample_metric_u64(entry, "process_peak_memory_kb");

    let cpu_total_ms = resources
        .and_then(|res| res.get("cpu_total_ms"))
        .and_then(json_value_to_u64)
        .or_else(|| {
            resources
                .and_then(|res| res.get("elapsed_cpu_ms"))
                .and_then(json_value_to_u64)
        })
        .or_else(|| {
            (!sample_cpu_ms.is_empty()).then(|| {
                sample_cpu_ms
                    .iter()
                    .fold(0_u128, |sum, value| sum.saturating_add(u128::from(*value)))
                    .min(u128::from(u64::MAX)) as u64
            })
        });
    let cpu_median_ms = resources
        .and_then(|res| res.get("cpu_median_ms"))
        .and_then(json_value_to_u64)
        .or_else(|| median_u64(&sample_cpu_ms));
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
        .or_else(|| sample_peak_memory_kb.iter().copied().max());
    let peak_memory_kb = peak_memory_growth_kb;
    let process_peak_memory_kb = resources
        .and_then(|res| res.get("process_peak_memory_kb"))
        .and_then(json_value_to_u64)
        .or_else(|| sample_process_peak_memory_kb.iter().copied().max());

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

pub(crate) fn render_markdown_summary(summary: &SummaryReport) -> String {
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

    let _ = writeln!(
        output,
        "| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |"
    );
    let _ = writeln!(
        output,
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
    );
    for device in &summary.device_summaries {
        for bench in &device.benchmarks {
            let _ = writeln!(
                output,
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                device.device,
                bench.function,
                bench.samples,
                summary.warmup,
                format_ms(bench.mean_ns),
                format_wall_total(bench.mean_ns, bench.samples),
                format_cpu_median_ms(bench.resource_usage.as_ref()),
                format_cpu_total_ms(bench.resource_usage.as_ref()),
                format_cpu_wall_ratio(bench.mean_ns, bench.samples, bench.resource_usage.as_ref()),
                format_peak_memory(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(BenchmarkResourceUsage::peak_memory_growth_or_legacy_kb)
                ),
                format_peak_memory(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(|usage| usage.process_peak_memory_kb)
                ),
            );
        }
    }
    let _ = writeln!(output);
    if summary_has_memory_baseline_gap(summary) {
        let _ = writeln!(output, "_Note: {}_", MEMORY_BASELINE_GAP_NOTE);
        let _ = writeln!(output);
    }

    output
}

pub(crate) fn render_csv_summary(summary: &SummaryReport) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "device,function,samples,mean_ns,median_ns,p95_ns,min_ns,max_ns,cpu_total_ms,cpu_median_ms,peak_memory_kb,peak_memory_growth_kb,process_peak_memory_kb"
    );
    for device in &summary.device_summaries {
        for bench in &device.benchmarks {
            let _ = writeln!(
                output,
                "{},{},{},{},{},{},{},{},{},{},{},{},{}",
                device.device,
                bench.function,
                bench.samples,
                bench.mean_ns.map_or(String::from(""), |v| v.to_string()),
                bench.median_ns.map_or(String::from(""), |v| v.to_string()),
                bench.p95_ns.map_or(String::from(""), |v| v.to_string()),
                bench.min_ns.map_or(String::from(""), |v| v.to_string()),
                bench.max_ns.map_or(String::from(""), |v| v.to_string()),
                bench
                    .resource_usage
                    .as_ref()
                    .and_then(|usage| usage.cpu_total_ms)
                    .map_or(String::new(), |v| v.to_string()),
                bench
                    .resource_usage
                    .as_ref()
                    .and_then(|usage| usage.cpu_median_ms)
                    .map_or(String::new(), |v| v.to_string()),
                bench
                    .resource_usage
                    .as_ref()
                    .and_then(|usage| usage.peak_memory_kb)
                    .map_or(String::new(), |v| v.to_string()),
                bench
                    .resource_usage
                    .as_ref()
                    .and_then(BenchmarkResourceUsage::peak_memory_growth_or_legacy_kb)
                    .map_or(String::new(), |v| v.to_string()),
                bench
                    .resource_usage
                    .as_ref()
                    .and_then(|usage| usage.process_peak_memory_kb)
                    .map_or(String::new(), |v| v.to_string())
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
pub(crate) fn format_duration_smart(ns: u64) -> String {
    let ms = ns as f64 / 1_000_000.0;
    if ms >= 1000.0 {
        // Convert to seconds
        let secs = ms / 1000.0;
        format!("{:.3}s", secs)
    } else {
        format!("{:.3}ms", ms)
    }
}

pub(crate) fn format_ms(value: Option<u64>) -> String {
    value
        .map(format_duration_smart)
        .unwrap_or_else(|| "-".to_string())
}

fn wall_total_ns(mean_ns: Option<u64>, samples: usize) -> Option<u64> {
    let mean_ns = u128::from(mean_ns?);
    let samples = u128::try_from(samples).ok()?;
    Some(mean_ns.saturating_mul(samples).min(u128::from(u64::MAX)) as u64)
}

fn format_wall_total(mean_ns: Option<u64>, samples: usize) -> String {
    wall_total_ns(mean_ns, samples)
        .map(format_duration_smart)
        .unwrap_or_else(|| "-".to_string())
}

fn format_cpu_median_ms(value: Option<&BenchmarkResourceUsage>) -> String {
    value
        .and_then(|usage| usage.cpu_median_ms)
        .map(format_cpu_total_duration_ms)
        .unwrap_or_else(|| "-".to_string())
}

fn format_cpu_total_ms(value: Option<&BenchmarkResourceUsage>) -> String {
    value
        .and_then(|usage| usage.cpu_total_ms)
        .map(format_cpu_total_duration_ms)
        .unwrap_or_else(|| "-".to_string())
}

fn format_cpu_wall_ratio(
    mean_ns: Option<u64>,
    samples: usize,
    value: Option<&BenchmarkResourceUsage>,
) -> String {
    let cpu_total_ms = value.and_then(|usage| usage.cpu_total_ms);
    match (wall_total_ns(mean_ns, samples), cpu_total_ms) {
        (Some(wall_total_ns), Some(cpu_total_ms)) if wall_total_ns > 0 => {
            let ratio = (cpu_total_ms as f64) / (wall_total_ns as f64 / 1_000_000.0) * 100.0;
            format!("{ratio:.1}%")
        }
        _ => "-".to_string(),
    }
}

pub(crate) fn format_cpu_total_duration_ms(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else {
        format!("{:.3}s", ms as f64 / 1_000.0)
    }
}

const MEMORY_BASELINE_GAP_MIN_DIFF_KB: u64 = 256 * 1024;
const MEMORY_BASELINE_GAP_RATIO: u64 = 4;
pub(crate) const MEMORY_BASELINE_GAP_NOTE: &str =
    "memory growth excludes warmup/baseline retained before the measured iteration.";

fn summary_has_memory_baseline_gap(summary: &SummaryReport) -> bool {
    summary.device_summaries.iter().any(|device| {
        device.benchmarks.iter().any(|benchmark| {
            benchmark
                .resource_usage
                .as_ref()
                .is_some_and(resource_usage_has_memory_baseline_gap)
        })
    })
}

fn resource_usage_has_memory_baseline_gap(usage: &BenchmarkResourceUsage) -> bool {
    let peak = usage.process_peak_memory_kb;
    match (usage.peak_memory_growth_or_legacy_kb(), peak) {
        (Some(growth), Some(peak)) if peak > growth => {
            peak.saturating_sub(growth) >= MEMORY_BASELINE_GAP_MIN_DIFF_KB
                && peak >= growth.saturating_mul(MEMORY_BASELINE_GAP_RATIO)
        }
        _ => false,
    }
}

fn format_peak_memory(value_kb: Option<u64>) -> String {
    value_kb
        .map(|value| format!("{:.2} MB", value as f64 / 1024.0))
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn cmd_summary(report_path: &Path, format: Option<SummaryFormat>) -> Result<()> {
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
