use crate::{
    BenchmarkFailureStats, BenchmarkResourceUsage, BenchmarkStats, CompareReport, CompareRow,
    RegressionFinding, SummaryReport, csv_field, markdown_inline_field_text,
    markdown_table_field_text,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fmt::{Display, Write};
use std::path::Path;

const MEMORY_BASELINE_GAP_MIN_DIFF_KB: u64 = 256 * 1024;
const MEMORY_BASELINE_GAP_RATIO: u64 = 4;
pub const MEMORY_BASELINE_GAP_NOTE: &str =
    "memory growth excludes warmup/baseline retained before the measured iteration.";

/// Render the released Markdown compatibility report from the canonical model.
#[must_use]
pub fn render_markdown_summary<T: Display>(summary: &SummaryReport<T>) -> String {
    let mut output = String::new();
    let devices = if summary.devices.is_empty() {
        "none".to_string()
    } else {
        summary
            .devices
            .iter()
            .map(|device| markdown_inline_field_text(device))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let _ = writeln!(output, "### Benchmark Summary");
    let _ = writeln!(output);
    let _ = writeln!(
        output,
        "- Generated: {}",
        markdown_inline_field_text(&summary.generated_at)
    );
    let _ = writeln!(
        output,
        "- Target: {}",
        markdown_inline_field_text(&summary.target.to_string())
    );
    let _ = writeln!(
        output,
        "- Function: {}",
        markdown_inline_field_text(&summary.function)
    );
    let _ = writeln!(
        output,
        "- Iterations/Warmup: {} / {}",
        summary.iterations, summary.warmup
    );
    let _ = writeln!(output, "- Devices: {devices}");
    let _ = writeln!(output);

    if summary.device_summaries.is_empty() {
        let _ = writeln!(output, "No benchmark samples were collected.");
        return output;
    }

    let has_failures = summary.device_summaries.iter().any(|device| {
        device
            .benchmarks
            .iter()
            .any(|benchmark| benchmark.failure.is_some())
    });
    if has_failures {
        let _ = writeln!(
            output,
            "| Device | Function | Status | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak | Elapsed | Exit reason |"
        );
        let _ = writeln!(
            output,
            "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"
        );
    } else {
        let _ = writeln!(
            output,
            "| Device | Function | Samples | Warmup | Wall mean / iter | Wall total | CPU median / iter | CPU total | CPU / wall | Peak growth | Process peak |"
        );
        let _ = writeln!(
            output,
            "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
        );
    }

    for device in &summary.device_summaries {
        for bench in &device.benchmarks {
            if has_failures {
                let _ = writeln!(
                    output,
                    "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                    markdown_table_field_text(&device.device),
                    markdown_table_field_text(&bench.function),
                    format_benchmark_status(bench),
                    bench.samples,
                    summary.warmup,
                    format_ms(bench.mean_ns),
                    format_wall_total(bench.mean_ns, bench.samples),
                    format_cpu_median_ms(bench.resource_usage.as_ref()),
                    format_cpu_total_ms(bench.resource_usage.as_ref()),
                    format_cpu_wall_ratio(
                        bench.mean_ns,
                        bench.samples,
                        bench.resource_usage.as_ref()
                    ),
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
                    format_failure_elapsed_ms(bench.failure.as_ref()),
                    markdown_table_field_text(
                        bench
                            .failure
                            .as_ref()
                            .and_then(|failure| failure.exit_reason.as_deref())
                            .unwrap_or("-")
                    ),
                );
            } else {
                let _ = writeln!(
                    output,
                    "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                    markdown_table_field_text(&device.device),
                    markdown_table_field_text(&bench.function),
                    bench.samples,
                    summary.warmup,
                    format_ms(bench.mean_ns),
                    format_wall_total(bench.mean_ns, bench.samples),
                    format_cpu_median_ms(bench.resource_usage.as_ref()),
                    format_cpu_total_ms(bench.resource_usage.as_ref()),
                    format_cpu_wall_ratio(
                        bench.mean_ns,
                        bench.samples,
                        bench.resource_usage.as_ref()
                    ),
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
    }
    let _ = writeln!(output);
    let diagnostics = summary
        .device_summaries
        .iter()
        .flat_map(|device| {
            device.benchmarks.iter().filter_map(move |benchmark| {
                benchmark.resource_usage.as_ref().and_then(|usage| {
                    (usage.logical_cpu_count.is_some()
                        || usage.affinity_cpu_count.is_some()
                        || usage.rayon_num_threads_env.is_some()
                        || usage.effective_cpu_cores_median.is_some())
                    .then_some((device, benchmark, usage))
                })
            })
        })
        .collect::<Vec<_>>();
    if !diagnostics.is_empty() {
        let _ = writeln!(output, "#### CPU diagnostics");
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "| Device | Function | Effective cores | Logical CPUs | Affinity CPUs | `RAYON_NUM_THREADS` |"
        );
        let _ = writeln!(output, "| --- | --- | ---: | ---: | ---: | ---: |");
        for (device, benchmark, usage) in diagnostics {
            let _ = writeln!(
                output,
                "| {} | {} | {} | {} | {} | {} |",
                markdown_table_field_text(&device.device),
                markdown_table_field_text(&benchmark.function),
                optional_float(usage.effective_cpu_cores_median),
                optional_number(usage.logical_cpu_count),
                optional_number(usage.affinity_cpu_count),
                optional_number(usage.rayon_num_threads_env),
            );
        }
        let _ = writeln!(output);
    }
    if summary_has_memory_baseline_gap(summary) {
        let _ = writeln!(output, "_Note: {MEMORY_BASELINE_GAP_NOTE}_");
        let _ = writeln!(output);
    }
    output
}

/// Render the released RFC 4180 CSV report from the canonical model.
#[must_use]
pub fn render_csv_summary<T>(summary: &SummaryReport<T>) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "device,function,samples,mean_ns,median_ns,p95_ns,min_ns,max_ns,cpu_total_ms,cpu_median_ms,peak_memory_kb,peak_memory_growth_kb,process_peak_memory_kb,effective_cpu_cores_median,logical_cpu_count,affinity_cpu_count,rayon_num_threads_env"
    );
    for device in &summary.device_summaries {
        for bench in &device.benchmarks {
            let _ = writeln!(
                output,
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                csv_field(&device.device),
                csv_field(&bench.function),
                bench.samples,
                optional_number(bench.mean_ns),
                optional_number(bench.median_ns),
                optional_number(bench.p95_ns),
                optional_number(bench.min_ns),
                optional_number(bench.max_ns),
                optional_number(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(|usage| usage.cpu_total_ms)
                ),
                optional_number(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(|usage| usage.cpu_median_ms)
                ),
                optional_number(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(|usage| usage.peak_memory_kb)
                ),
                optional_number(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(BenchmarkResourceUsage::peak_memory_growth_or_legacy_kb)
                ),
                optional_number(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(|usage| usage.process_peak_memory_kb)
                ),
                optional_float(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(|usage| usage.effective_cpu_cores_median)
                ),
                optional_number(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(|usage| usage.logical_cpu_count)
                ),
                optional_number(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(|usage| usage.affinity_cpu_count)
                ),
                optional_number(
                    bench
                        .resource_usage
                        .as_ref()
                        .and_then(|usage| usage.rayon_num_threads_env)
                ),
            );
        }
    }
    output
}

/// Compare two canonical summary models in deterministic device/function order.
#[must_use]
pub fn compare_summaries<T, U>(
    baseline_path: &Path,
    candidate_path: &Path,
    baseline: &SummaryReport<T>,
    candidate: &SummaryReport<U>,
) -> CompareReport {
    let baseline_map = summary_lookup(baseline);
    let candidate_map = summary_lookup(candidate);
    let mut rows = Vec::new();
    let mut devices = BTreeMap::new();
    devices.extend(baseline_map.keys().map(|key| (key.clone(), ())));
    devices.extend(candidate_map.keys().map(|key| (key.clone(), ())));

    for device in devices.keys() {
        let mut functions = BTreeMap::new();
        if let Some(entry) = baseline_map.get(device) {
            functions.extend(entry.keys().map(|key| (key.clone(), ())));
        }
        if let Some(entry) = candidate_map.get(device) {
            functions.extend(entry.keys().map(|key| (key.clone(), ())));
        }
        for function in functions.keys() {
            let baseline_stats = baseline_map
                .get(device)
                .and_then(|entry| entry.get(function));
            let candidate_stats = candidate_map
                .get(device)
                .and_then(|entry| entry.get(function));
            let baseline_median_ns = baseline_stats.and_then(|stats| stats.median_ns);
            let candidate_median_ns = candidate_stats.and_then(|stats| stats.median_ns);
            let median_delta_pct = percent_delta(baseline_median_ns, candidate_median_ns);
            let baseline_p95_ns = baseline_stats.and_then(|stats| stats.p95_ns);
            let candidate_p95_ns = candidate_stats.and_then(|stats| stats.p95_ns);
            let p95_delta_pct = percent_delta(baseline_p95_ns, candidate_p95_ns);
            rows.push(CompareRow {
                device: device.clone(),
                function: function.clone(),
                baseline_median_ns,
                candidate_median_ns,
                median_delta_pct,
                median_label: delta_label(median_delta_pct, 0.0).to_string(),
                baseline_p95_ns,
                candidate_p95_ns,
                p95_delta_pct,
                p95_label: delta_label(p95_delta_pct, 0.0).to_string(),
            });
        }
    }
    CompareReport {
        baseline: baseline_path.to_path_buf(),
        candidate: candidate_path.to_path_buf(),
        rows,
    }
}

#[must_use]
pub fn detect_regressions(report: &CompareReport, threshold_pct: f64) -> Vec<RegressionFinding> {
    let mut findings = Vec::new();
    for row in &report.rows {
        for (metric, delta) in [("median", row.median_delta_pct), ("p95", row.p95_delta_pct)] {
            if let Some(delta_pct) = delta.filter(|value| *value > threshold_pct) {
                findings.push(RegressionFinding {
                    device: row.device.clone(),
                    function: row.function.clone(),
                    metric: metric.to_string(),
                    delta_pct,
                });
            }
        }
    }
    findings
}

#[must_use]
pub fn render_compare_markdown(report: &CompareReport) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "### Benchmark Comparison");
    let _ = writeln!(output);
    let _ = writeln!(
        output,
        "- Baseline: {}",
        markdown_inline_field_text(&report.baseline.display().to_string())
    );
    let _ = writeln!(
        output,
        "- Candidate: {}",
        markdown_inline_field_text(&report.candidate.display().to_string())
    );
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
            markdown_table_field_text(&row.device),
            markdown_table_field_text(&row.function),
            format_ms(row.baseline_median_ns),
            format_ms(row.candidate_median_ns),
            format_delta(row.median_delta_pct),
            markdown_table_field_text(&row.median_label),
            format_ms(row.baseline_p95_ns),
            format_ms(row.candidate_p95_ns),
            format_delta(row.p95_delta_pct),
            markdown_table_field_text(&row.p95_label),
        );
    }
    output
}

/// Render JUnit for CI consumers from the canonical model and regression set.
#[must_use]
pub fn render_junit_report<T>(
    summary: &SummaryReport<T>,
    regressions: &[RegressionFinding],
) -> String {
    let mut output = String::new();
    let mut failures_by_case: HashMap<(String, String), Vec<&RegressionFinding>> = HashMap::new();
    for finding in regressions {
        failures_by_case
            .entry((finding.device.clone(), finding.function.clone()))
            .or_default()
            .push(finding);
    }
    let total_tests = summary
        .device_summaries
        .iter()
        .map(|device| device.benchmarks.len())
        .sum::<usize>();
    let total_failures = summary
        .device_summaries
        .iter()
        .flat_map(|device| device.benchmarks.iter().map(move |bench| (device, bench)))
        .filter(|(device, bench)| {
            failures_by_case.contains_key(&(device.device.clone(), bench.function.clone()))
        })
        .count();

    let _ = writeln!(output, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        output,
        r#"<testsuite name="mobench" tests="{total_tests}" failures="{total_failures}">"#
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
                r#"  <testcase name="{}" classname="{}" time="{time_secs:.6}">"#,
                escape_xml(&case_name),
                escape_xml(&device.device)
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

/// JSON projection used by compatibility summaries and GitHub adapters.
#[must_use]
pub fn comparison_json(
    report: &CompareReport,
    threshold_pct: f64,
    baseline_source: Option<&str>,
) -> Value {
    json!({
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
    })
}

fn summary_lookup<T>(
    summary: &SummaryReport<T>,
) -> BTreeMap<String, BTreeMap<String, BenchmarkStats>> {
    summary
        .device_summaries
        .iter()
        .map(|device| {
            (
                device.device.clone(),
                device
                    .benchmarks
                    .iter()
                    .map(|bench| (bench.function.clone(), bench.clone()))
                    .collect(),
            )
        })
        .collect()
}

fn percent_delta(baseline: Option<u64>, candidate: Option<u64>) -> Option<f64> {
    let baseline = baseline? as f64;
    let candidate = candidate? as f64;
    (baseline != 0.0).then_some(((candidate - baseline) / baseline) * 100.0)
}

fn delta_label(delta: Option<f64>, threshold_pct: f64) -> &'static str {
    match delta {
        Some(value) if value >= threshold_pct => "regressed",
        Some(value) if value <= -threshold_pct => "improved",
        _ => "neutral",
    }
}

fn optional_number<T: Display>(value: Option<T>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn format_benchmark_status(bench: &BenchmarkStats) -> String {
    bench.failure.as_ref().map_or_else(
        || "ok".to_string(),
        |failure| format!("failed ({})", markdown_table_field_text(&failure.kind)),
    )
}

#[must_use]
pub fn format_failure_elapsed_ms(failure: Option<&BenchmarkFailureStats>) -> String {
    failure
        .and_then(|failure| failure.elapsed_ms)
        .map(|elapsed_ms| format!("{:.3}s", elapsed_ms as f64 / 1_000.0))
        .unwrap_or_else(|| "-".to_string())
}

#[must_use]
pub fn format_duration_smart(ns: u64) -> String {
    let ms = ns as f64 / 1_000_000.0;
    if ms >= 1_000.0 {
        format!("{:.3}s", ms / 1_000.0)
    } else {
        format!("{ms:.3}ms")
    }
}

#[must_use]
pub fn format_ms(value: Option<u64>) -> String {
    value
        .map(format_duration_smart)
        .unwrap_or_else(|| "-".to_string())
}

fn wall_total_ns(mean_ns: Option<u64>, samples: usize) -> Option<u64> {
    let total = u128::from(mean_ns?).saturating_mul(u128::try_from(samples).ok()?);
    Some(total.min(u128::from(u64::MAX)) as u64)
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
    match (
        wall_total_ns(mean_ns, samples),
        value.and_then(|usage| usage.cpu_total_ms),
    ) {
        (Some(wall_ns), Some(cpu_ms)) if wall_ns > 0 => format!(
            "{:.1}%",
            cpu_ms as f64 / (wall_ns as f64 / 1_000_000.0) * 100.0
        ),
        _ => "-".to_string(),
    }
}

fn optional_float(value: Option<f64>) -> String {
    value.map(|value| format!("{value:.3}")).unwrap_or_default()
}

#[must_use]
pub fn format_cpu_total_duration_ms(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else {
        format!("{:.3}s", ms as f64 / 1_000.0)
    }
}

fn format_peak_memory(value_kb: Option<u64>) -> String {
    value_kb
        .map(|value| format!("{:.2} MB", value as f64 / 1_024.0))
        .unwrap_or_else(|| "-".to_string())
}

fn summary_has_memory_baseline_gap<T>(summary: &SummaryReport<T>) -> bool {
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
    match (
        usage.peak_memory_growth_or_legacy_kb(),
        usage.process_peak_memory_kb,
    ) {
        (Some(growth), Some(peak)) if peak > growth => {
            peak.saturating_sub(growth) >= MEMORY_BASELINE_GAP_MIN_DIFF_KB
                && peak >= growth.saturating_mul(MEMORY_BASELINE_GAP_RATIO)
        }
        _ => false,
    }
}

fn format_delta(value: Option<f64>) -> String {
    value
        .map(|delta| format!("{delta:+.2}%"))
        .unwrap_or_else(|| "-".to_string())
}

fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BenchmarkStats, DeviceSummary};

    fn summary() -> SummaryReport<&'static str> {
        SummaryReport {
            generated_at: "2026-07-16T00:00:00Z".to_string(),
            generated_at_unix: 1,
            target: "Android",
            function: "crate::bench".to_string(),
            iterations: 3,
            warmup: 1,
            devices: vec!["Pixel 7".to_string()],
            device_summaries: vec![DeviceSummary {
                device: "Pixel 7".to_string(),
                benchmarks: vec![BenchmarkStats {
                    function: "crate::bench".to_string(),
                    samples: 3,
                    mean_ns: Some(2_000_000),
                    median_ns: Some(2_000_000),
                    p95_ns: Some(3_000_000),
                    min_ns: Some(1_000_000),
                    max_ns: Some(3_000_000),
                    resource_usage: None,
                    failure: None,
                }],
            }],
        }
    }

    #[test]
    fn all_primary_adapters_render_one_canonical_summary() {
        let summary = summary();
        assert!(render_markdown_summary(&summary).contains("Pixel 7"));
        assert!(render_csv_summary(&summary).contains("Pixel 7,crate::bench,3"));
        assert!(render_junit_report(&summary, &[]).contains("tests=\"1\""));
    }

    #[test]
    fn comparison_order_and_regression_gate_are_deterministic() {
        let baseline = summary();
        let mut candidate = summary();
        candidate.device_summaries[0].benchmarks[0].median_ns = Some(3_000_000);
        let report = compare_summaries(
            Path::new("base.json"),
            Path::new("candidate.json"),
            &baseline,
            &candidate,
        );
        assert_eq!(report.rows.len(), 1);
        assert_eq!(detect_regressions(&report, 10.0).len(), 1);
        assert!(render_compare_markdown(&report).contains("+50.00%"));
    }

    #[test]
    fn markdown_cpu_diagnostics_include_effective_core_count() {
        let mut summary = summary();
        summary.device_summaries[0].benchmarks[0].resource_usage = Some(BenchmarkResourceUsage {
            effective_cpu_cores_median: Some(3.75),
            logical_cpu_count: Some(8),
            affinity_cpu_count: Some(6),
            rayon_num_threads_env: Some(4),
            ..BenchmarkResourceUsage::default()
        });

        let markdown = render_markdown_summary(&summary);
        assert!(markdown.contains("| Device | Function | Effective cores |"));
        assert!(markdown.contains("| Pixel 7 | crate::bench | 3.750 | 8 | 6 | 4 |"));
    }
}
