use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::reports::{ensure_parent_dir, format_ms};
use crate::{BenchmarkStats, RunSummary, SummaryReport, repo_root, write_file};

#[derive(Debug, Clone)]
pub(crate) struct RegressionFinding {
    pub(crate) device: String,
    pub(crate) function: String,
    pub(crate) metric: String,
    pub(crate) delta_pct: f64,
}

pub(crate) fn detect_regressions(
    report: &CompareReport,
    threshold_pct: f64,
) -> Vec<RegressionFinding> {
    let mut findings = Vec::new();
    for row in &report.rows {
        if let Some(delta) = row.median_delta_pct
            && delta > threshold_pct
        {
            findings.push(RegressionFinding {
                device: row.device.clone(),
                function: row.function.clone(),
                metric: "median".to_string(),
                delta_pct: delta,
            });
        }
        if let Some(delta) = row.p95_delta_pct
            && delta > threshold_pct
        {
            findings.push(RegressionFinding {
                device: row.device.clone(),
                function: row.function.clone(),
                metric: "p95".to_string(),
                delta_pct: delta,
            });
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

fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[derive(Debug, Serialize)]
pub(crate) struct CompareReport {
    pub(crate) baseline: PathBuf,
    pub(crate) candidate: PathBuf,
    pub(crate) rows: Vec<CompareRow>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CompareRow {
    pub(crate) device: String,
    pub(crate) function: String,
    pub(crate) baseline_median_ns: Option<u64>,
    pub(crate) candidate_median_ns: Option<u64>,
    pub(crate) median_delta_pct: Option<f64>,
    pub(crate) median_label: String,
    pub(crate) baseline_p95_ns: Option<u64>,
    pub(crate) candidate_p95_ns: Option<u64>,
    pub(crate) p95_delta_pct: Option<f64>,
    pub(crate) p95_label: String,
}

pub(crate) fn compare_summaries(baseline: &Path, candidate: &Path) -> Result<CompareReport> {
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

pub(crate) fn inject_compare_into_summary(
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

pub(crate) fn render_compare_markdown(report: &CompareReport) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "### Benchmark Comparison");
    let _ = writeln!(output);
    let _ = writeln!(output, "- Baseline: {}", report.baseline.display());
    let _ = writeln!(output, "- Candidate: {}", report.candidate.display());
    let _ = writeln!(output);
    let _ = writeln!(
        output,
        "| Device | Function | Median base | Median cand | Median Δ% | Median Label | P95 base | P95 cand | P95 Δ% | P95 Label |"
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

fn format_delta(value: Option<f64>) -> String {
    value
        .map(|delta| format!("{:+.2}%", delta))
        .unwrap_or_else(|| "-".to_string())
}
