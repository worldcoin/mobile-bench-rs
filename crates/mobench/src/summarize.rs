//! Types and logic for the `ci summarize` command.

use anyhow::{Context, Result};
use comfy_table::{Attribute, Cell, ContentArrangement, Table, presets::UTF8_FULL};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// A fully-assembled summary ready for rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizeReport {
    pub platforms: Vec<PlatformReport>,
}

/// Results for a single platform (iOS or Android).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformReport {
    pub platform: String,
    pub device: DeviceInfo,
    pub benchmarks: Vec<BenchmarkResult>,
    pub iterations: u32,
    pub warmup: u32,
}

/// Device information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub os: String,
    pub os_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chipset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_gb: Option<f64>,
}

/// Aggregated result for a single benchmark function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub name: String,
    pub label: String,
    pub timing: TimingStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_usage: Option<ResourceUsage>,
}

/// Timing statistics across all iterations (in milliseconds).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingStats {
    pub avg_ms: f64,
    pub median_ms: f64,
    pub best_ms: f64,
    pub worst_ms: f64,
    pub p95_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub std_dev_ms: Option<f64>,
}

/// Resource usage metrics from BrowserStack session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_total_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_memory_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_pss_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_dirty_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_heap_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_heap_kb: Option<u64>,
}

/// Parse a summary.json value into a [`SummarizeReport`].
pub fn parse_summary_value(value: &serde_json::Value) -> Result<SummarizeReport> {
    if let Some(summary) = value.get("summary") {
        return parse_summary_object(summary);
    }

    if let Some(targets) = value.get("targets").and_then(|v| v.as_object()) {
        let mut platforms = Vec::new();
        for entry in targets.values() {
            if let Ok(report) = parse_summary_value(entry) {
                platforms.extend(report.platforms);
            }
        }
        if !platforms.is_empty() {
            return Ok(SummarizeReport { platforms });
        }
    }

    anyhow::bail!("Missing 'summary' or 'targets' key in JSON");
}

fn parse_summary_object(summary: &serde_json::Value) -> Result<SummarizeReport> {
    let summary = summary
        .as_object()
        .context("Summary payload must be a JSON object")?;

    let target = summary
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let iterations = summary
        .get("iterations")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let warmup = summary.get("warmup").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

    let device_summaries = summary
        .get("device_summaries")
        .and_then(|v| v.as_array())
        .context("Missing 'device_summaries'")?;

    let mut platforms = Vec::new();

    for ds in device_summaries {
        let device_str = ds
            .get("device")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let device = parse_device_string(device_str);

        let benchmarks = ds
            .get("benchmarks")
            .and_then(|v| v.as_array())
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|b| parse_benchmark_entry(b).ok())
            .collect();

        platforms.push(PlatformReport {
            platform: target.clone(),
            device,
            benchmarks,
            iterations,
            warmup,
        });
    }

    Ok(SummarizeReport { platforms })
}

fn parse_device_string(s: &str) -> DeviceInfo {
    let (name, os_version) = match s.rsplit_once('-') {
        Some((n, v)) => (n.to_string(), v.to_string()),
        None => (s.to_string(), "unknown".to_string()),
    };

    let os = if name.contains("iPhone") || name.contains("iPad") {
        "iOS".to_string()
    } else {
        "Android".to_string()
    };

    DeviceInfo {
        name,
        os,
        os_version,
        chipset: None,
        ram_gb: None,
    }
}

fn parse_benchmark_entry(value: &serde_json::Value) -> Result<BenchmarkResult> {
    let name = value
        .get("function")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let label = humanize_benchmark_name(&name);

    let ns_to_ms =
        |key: &str| -> f64 { value.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0) / 1_000_000.0 };

    let timing = TimingStats {
        avg_ms: ns_to_ms("mean_ns"),
        median_ms: ns_to_ms("median_ns"),
        best_ms: ns_to_ms("min_ns"),
        worst_ms: ns_to_ms("max_ns"),
        p95_ms: ns_to_ms("p95_ns"),
        std_dev_ms: value
            .get("std_dev_ns")
            .and_then(|v| v.as_f64())
            .filter(|&v| v > 0.0)
            .map(|v| v / 1_000_000.0),
    };

    Ok(BenchmarkResult {
        name,
        label,
        timing,
        resource_usage: parse_resource_usage(value),
    })
}

/// Produce a human-friendly label from a fully-qualified benchmark name.
///
/// Strips leading `bench_` and converts underscores to hyphens.
/// The raw function name is always preserved in [`BenchmarkResult::name`].
fn humanize_benchmark_name(name: &str) -> String {
    // Use the last segment if qualified (e.g. "crate::bench_foo" → "bench_foo")
    let leaf = name.rsplit("::").next().unwrap_or(name);
    let s = leaf.strip_prefix("bench_").unwrap_or(leaf);
    s.replace('_', "-")
}

/// Load all summary JSON files from a results directory.
pub fn load_results_dir(dir: &Path) -> Result<SummarizeReport> {
    let root_summary_path = dir.join("summary.json");
    let mut enrichment_values = Vec::new();
    let mut root_report = None;
    if root_summary_path.exists() {
        let content = std::fs::read_to_string(&root_summary_path)
            .with_context(|| format!("Failed to read {}", root_summary_path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", root_summary_path.display()))?;
        if let Ok(report) = parse_summary_value(&value) {
            root_report = Some(report);
            enrichment_values.push(value);
        }
    }

    let mut json_paths = Vec::new();
    collect_json_files(dir, &mut json_paths)?;
    json_paths.retain(|path| path != &root_summary_path);
    json_paths.sort_by(compare_summary_candidates);

    let mut all_platforms = root_report
        .map(|report| report.platforms)
        .unwrap_or_default();
    let mut raw_candidates = Vec::new();
    let mut covered_summary_dirs = if all_platforms.is_empty() {
        Vec::new()
    } else {
        vec![dir.to_path_buf()]
    };

    for path in json_paths {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;

        if let Ok(report) = parse_summary_value(&value) {
            enrichment_values.push(value.clone());
            let summary_dir = path.parent().unwrap_or(dir);
            if is_covered_by_canonical_summary(summary_dir, &covered_summary_dirs) {
                continue;
            }
            covered_summary_dirs.push(summary_dir.to_path_buf());
            all_platforms.extend(report.platforms);
        } else {
            raw_candidates.push((path, value));
        }
    }

    if all_platforms.is_empty() {
        for (path, value) in raw_candidates {
            if let Ok(report) = parse_raw_bench_report(&path, &value) {
                all_platforms.extend(report.platforms);
            }
        }
    }

    if all_platforms.is_empty() {
        anyhow::bail!("No valid summary JSON files found in {}", dir.display());
    }

    let mut report = SummarizeReport {
        platforms: all_platforms,
    };

    for value in &enrichment_values {
        enrich_report_with_summary_value(&mut report, value);
    }

    Ok(report)
}

const MAX_DIR_DEPTH: usize = 10;

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    collect_json_files_inner(dir, out, 0)
}

fn collect_json_files_inner(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) -> Result<()> {
    if depth > MAX_DIR_DEPTH {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read results directory {}", dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("Failed to iterate results directory {}", dir.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            collect_json_files_inner(&path, out, depth + 1)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }

    Ok(())
}

fn compare_summary_candidates(left: &PathBuf, right: &PathBuf) -> Ordering {
    left.components()
        .count()
        .cmp(&right.components().count())
        .then_with(|| {
            let left_is_summary =
                left.file_name().and_then(|name| name.to_str()) == Some("summary.json");
            let right_is_summary =
                right.file_name().and_then(|name| name.to_str()) == Some("summary.json");
            right_is_summary.cmp(&left_is_summary)
        })
        .then_with(|| left.cmp(right))
}

fn is_covered_by_canonical_summary(path: &Path, canonical_dirs: &[PathBuf]) -> bool {
    canonical_dirs
        .iter()
        .any(|dir| path == dir.as_path() || path.starts_with(dir))
}

fn enrich_report_with_summary_value(report: &mut SummarizeReport, value: &serde_json::Value) {
    if let Ok(nested_report) = parse_summary_value(value) {
        merge_resource_usage_from_report(report, &nested_report);
    }

    let target = value
        .get("summary")
        .and_then(|summary| summary.get("target"))
        .or_else(|| value.get("target"))
        .and_then(|target| target.as_str());
    let benchmark_results = value
        .get("benchmark_results")
        .and_then(|results| results.as_object());

    let (Some(target), Some(benchmark_results)) = (target, benchmark_results) else {
        return;
    };

    for (device_name, entries) in benchmark_results {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(function) = entry.get("function").and_then(|function| function.as_str())
            else {
                continue;
            };
            let Some(resource_usage) = parse_resource_usage(entry) else {
                continue;
            };
            set_benchmark_resource_usage(report, target, device_name, function, resource_usage);
        }
    }
}

fn merge_resource_usage_from_report(report: &mut SummarizeReport, nested_report: &SummarizeReport) {
    for nested_platform in &nested_report.platforms {
        for nested_benchmark in &nested_platform.benchmarks {
            let Some(resource_usage) = nested_benchmark.resource_usage.clone() else {
                continue;
            };
            set_benchmark_resource_usage(
                report,
                &nested_platform.platform,
                &nested_platform.device.name,
                &nested_benchmark.name,
                resource_usage,
            );
        }
    }
}

fn set_benchmark_resource_usage(
    report: &mut SummarizeReport,
    target: &str,
    device_name: &str,
    function: &str,
    resource_usage: ResourceUsage,
) {
    for platform in &mut report.platforms {
        if platform.platform != target || !device_names_match(&platform.device.name, device_name) {
            continue;
        }
        if let Some(benchmark) = platform
            .benchmarks
            .iter_mut()
            .find(|benchmark| benchmark.name == function)
        {
            if let Some(existing) = &mut benchmark.resource_usage {
                existing.merge_missing(&resource_usage);
            } else {
                benchmark.resource_usage = Some(resource_usage.clone());
            }
        }
    }
}

pub(crate) fn device_names_match(left: &str, right: &str) -> bool {
    let left_name = parse_device_string(left).name;
    let right_name = parse_device_string(right).name;
    left_name == right_name
}

fn session_matches_platform_row(
    platform: &PlatformReport,
    session: &crate::browserstack::SessionSummary,
) -> bool {
    let session_is_ios = session.os.eq_ignore_ascii_case("ios")
        || session.os.eq_ignore_ascii_case("iPhone")
        || session.os.eq_ignore_ascii_case("iPad");
    let platform_is_ios = platform.platform == "ios";

    if session_is_ios != platform_is_ios {
        return false;
    }

    if platform.device.name != "unknown"
        && !device_names_match(&platform.device.name, &session.device)
    {
        return false;
    }

    if platform.device.os_version != "unknown"
        && !platform.device.os_version.is_empty()
        && !session.os_version.is_empty()
        && platform.device.os_version != session.os_version
    {
        return false;
    }

    true
}

fn parse_raw_bench_report(path: &Path, value: &serde_json::Value) -> Result<SummarizeReport> {
    let entries = match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(normalize_raw_benchmark_entry)
            .collect::<Vec<_>>(),
        _ => normalize_raw_benchmark_entry(value).into_iter().collect(),
    };

    if entries.is_empty() {
        anyhow::bail!("Not a raw bench report");
    }

    let first = entries
        .first()
        .ok_or_else(|| anyhow::anyhow!("missing bench report entries"))?;
    let platform = infer_platform(path, first.get("device").and_then(|v| v.as_str()));
    let device = first
        .get("device")
        .and_then(|v| v.as_str())
        .map(parse_device_string)
        .unwrap_or_else(|| default_device_info(&platform));
    let iterations = first
        .get("spec")
        .and_then(|spec| spec.get("iterations"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let warmup = first
        .get("spec")
        .and_then(|spec| spec.get("warmup"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let benchmarks = entries
        .iter()
        .map(parse_benchmark_entry)
        .collect::<Result<Vec<_>>>()?;

    Ok(SummarizeReport {
        platforms: vec![PlatformReport {
            platform,
            device,
            benchmarks,
            iterations,
            warmup,
        }],
    })
}

fn normalize_raw_benchmark_entry(value: &serde_json::Value) -> Option<serde_json::Value> {
    let mut value = value.clone();
    let samples = extract_raw_samples(&value);
    let stats = crate::compute_sample_stats(&samples);
    let object = value.as_object_mut()?;

    if !object.contains_key("function")
        && let Some(function) = object
            .get("spec")
            .and_then(|spec| spec.get("name"))
            .and_then(|name| name.as_str())
    {
        object.insert(
            "function".to_string(),
            serde_json::Value::String(function.to_string()),
        );
    }

    if !object.contains_key("samples")
        && let Some(samples_ns) = object.get("samples_ns").and_then(|v| v.as_array())
    {
        object.insert(
            "samples".to_string(),
            serde_json::Value::Array(samples_ns.clone()),
        );
    }

    if let Some(stats) = stats {
        if !object.contains_key("mean_ns") {
            object.insert(
                "mean_ns".to_string(),
                serde_json::Value::from(stats.mean_ns),
            );
        }
        if !object.contains_key("median_ns") {
            object.insert(
                "median_ns".to_string(),
                serde_json::Value::from(stats.median_ns),
            );
        }
        if !object.contains_key("min_ns") {
            object.insert("min_ns".to_string(), serde_json::Value::from(stats.min_ns));
        }
        if !object.contains_key("max_ns") {
            object.insert("max_ns".to_string(), serde_json::Value::from(stats.max_ns));
        }
        if !object.contains_key("p95_ns") {
            object.insert("p95_ns".to_string(), serde_json::Value::from(stats.p95_ns));
        }
    }

    let has_function = object.get("function").and_then(|v| v.as_str()).is_some();
    let has_samples = object.get("samples").and_then(|v| v.as_array()).is_some();
    if has_function && has_samples {
        Some(value)
    } else {
        None
    }
}

fn extract_raw_samples(value: &serde_json::Value) -> Vec<u64> {
    let mut samples = crate::extract_samples(value);
    if samples.is_empty()
        && let Some(samples_ns) = value.get("samples_ns").and_then(|v| v.as_array())
    {
        samples.extend(samples_ns.iter().filter_map(|sample| sample.as_u64()));
    }
    samples
}

fn infer_platform(path: &Path, device: Option<&str>) -> String {
    if let Some(device) = device {
        let lower = device.to_ascii_lowercase();
        if lower.contains("iphone") || lower.contains("ipad") || lower.contains("ios") {
            return "ios".to_string();
        }
        if lower.contains("pixel") || lower.contains("android") {
            return "android".to_string();
        }
    }

    let lower_path = path.to_string_lossy().to_ascii_lowercase();
    if lower_path.contains("/ios/") || lower_path.contains("\\ios\\") {
        "ios".to_string()
    } else if lower_path.contains("/android/") || lower_path.contains("\\android\\") {
        "android".to_string()
    } else {
        "unknown".to_string()
    }
}

fn default_device_info(platform: &str) -> DeviceInfo {
    let os = match platform {
        "ios" => "iOS",
        "android" => "Android",
        _ => "unknown",
    };

    DeviceInfo {
        name: "unknown".to_string(),
        os: os.to_string(),
        os_version: "unknown".to_string(),
        chipset: None,
        ram_gb: None,
    }
}

impl ResourceUsage {
    fn is_empty(&self) -> bool {
        self.cpu_total_ms.is_none()
            && self.peak_memory_kb.is_none()
            && self.total_pss_kb.is_none()
            && self.private_dirty_kb.is_none()
            && self.native_heap_kb.is_none()
            && self.java_heap_kb.is_none()
    }

    fn merge_missing(&mut self, other: &Self) {
        if self.cpu_total_ms.is_none() {
            self.cpu_total_ms = other.cpu_total_ms;
        }
        if self.peak_memory_kb.is_none() {
            self.peak_memory_kb = other.peak_memory_kb;
        }
        if self.total_pss_kb.is_none() {
            self.total_pss_kb = other.total_pss_kb;
        }
        if self.private_dirty_kb.is_none() {
            self.private_dirty_kb = other.private_dirty_kb;
        }
        if self.native_heap_kb.is_none() {
            self.native_heap_kb = other.native_heap_kb;
        }
        if self.java_heap_kb.is_none() {
            self.java_heap_kb = other.java_heap_kb;
        }
    }
}

fn parse_resource_usage(value: &serde_json::Value) -> Option<ResourceUsage> {
    value
        .get("resource_usage")
        .and_then(parse_resource_usage_object)
        .or_else(|| value.get("resources").and_then(parse_resource_usage_object))
}

fn parse_resource_usage_object(value: &serde_json::Value) -> Option<ResourceUsage> {
    let object = value.as_object()?;

    let cpu_total_ms = object
        .get("cpu_total_ms")
        .or_else(|| object.get("elapsed_cpu_ms"))
        .and_then(json_value_to_u64);
    let total_pss_kb = object.get("total_pss_kb").and_then(json_value_to_u64);
    let private_dirty_kb = object.get("private_dirty_kb").and_then(json_value_to_u64);
    let native_heap_kb = object.get("native_heap_kb").and_then(json_value_to_u64);
    let java_heap_kb = object.get("java_heap_kb").and_then(json_value_to_u64);

    let peak_memory_kb = object
        .get("peak_memory_kb")
        .and_then(json_value_to_u64)
        .or_else(|| {
            object
                .get("ram_peak_mb")
                .and_then(|value| value.as_f64())
                .map(|value| (value * 1024.0).round() as u64)
        })
        .or_else(|| {
            raw_peak_memory_kb(total_pss_kb, private_dirty_kb, native_heap_kb, java_heap_kb)
        });

    let resource_usage = ResourceUsage {
        cpu_total_ms,
        peak_memory_kb,
        total_pss_kb,
        private_dirty_kb,
        native_heap_kb,
        java_heap_kb,
    };

    (!resource_usage.is_empty()).then_some(resource_usage)
}

fn json_value_to_u64(value: &serde_json::Value) -> Option<u64> {
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

fn format_cpu_total_ms(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn format_peak_memory(value_kb: Option<u64>) -> String {
    value_kb
        .map(|value| format!("{:.2} MB", value as f64 / 1024.0))
        .unwrap_or_else(|| "—".to_string())
}

/// Render the full report as terminal tables.
pub fn render_table(report: &SummarizeReport) -> String {
    let mut output = String::new();

    for platform in &report.platforms {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&render_platform_table(platform));
    }

    output
}

fn render_platform_table(platform: &PlatformReport) -> String {
    let mut output = String::new();

    // Header line
    let mut header = format!(
        "{} — {} ({} {})",
        platform.platform.to_uppercase(),
        platform.device.name,
        platform.device.os,
        platform.device.os_version,
    );
    if let Some(chipset) = &platform.device.chipset {
        header.push_str(&format!(" · {chipset}"));
    }
    if let Some(ram) = platform.device.ram_gb {
        header.push_str(&format!(" · {ram} GB RAM"));
    }
    output.push_str(&header);
    output.push('\n');

    let has_resource_usage = platform
        .benchmarks
        .iter()
        .any(|b| b.resource_usage.is_some());

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);

    let mut headers = vec!["Benchmark", "Avg ms", "Best", "Worst", "Median", "P95"];
    if has_resource_usage {
        headers.extend(["CPU total (ms)", "Peak memory"]);
    }
    table.set_header(
        headers
            .iter()
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
    );

    for bench in &platform.benchmarks {
        let mut row = vec![
            Cell::new(&bench.label),
            Cell::new(format!("{:.1}", bench.timing.avg_ms)).add_attribute(Attribute::Bold),
            Cell::new(format!("{:.1}", bench.timing.best_ms)),
            Cell::new(format!("{:.1}", bench.timing.worst_ms)),
            Cell::new(format!("{:.1}", bench.timing.median_ms)),
            Cell::new(format!("{:.1}", bench.timing.p95_ms)),
        ];

        if has_resource_usage {
            if let Some(ru) = &bench.resource_usage {
                row.push(Cell::new(format_cpu_total_ms(ru.cpu_total_ms)));
                row.push(Cell::new(format_peak_memory(ru.peak_memory_kb)));
            } else {
                row.push(Cell::new("—"));
                row.push(Cell::new("—"));
            }
        }

        table.add_row(row);
    }

    output.push_str(&table.to_string());
    output.push_str(&format!(
        "\n  {} iterations · {} warmup · avg is primary metric\n",
        platform.iterations, platform.warmup
    ));

    output
}

/// Render the report as a markdown table.
pub fn render_markdown(report: &SummarizeReport) -> String {
    let mut output = String::new();

    for platform in &report.platforms {
        if !output.is_empty() {
            output.push('\n');
        }

        let mut header = format!(
            "### {} — {} ({} {})",
            platform.platform.to_uppercase(),
            platform.device.name,
            platform.device.os,
            platform.device.os_version,
        );
        if let Some(chipset) = &platform.device.chipset {
            header.push_str(&format!(" · {chipset}"));
        }
        if let Some(ram) = platform.device.ram_gb {
            header.push_str(&format!(" · {ram} GB RAM"));
        }
        output.push_str(&header);
        output.push_str("\n\n");

        let has_ru = platform
            .benchmarks
            .iter()
            .any(|b| b.resource_usage.is_some());

        if has_ru {
            output.push_str(
                "| Benchmark | Avg ms | Best | Worst | Median | P95 | CPU total (ms) | Peak memory |\n",
            );
            output.push_str(
                "|-----------|--------|------|-------|--------|-----|----------------|-------------|\n",
            );
        } else {
            output.push_str("| Benchmark | Avg ms | Best | Worst | Median | P95 |\n");
            output.push_str("|-----------|--------|------|-------|--------|-----|\n");
        }

        for bench in &platform.benchmarks {
            let mut row = format!(
                "| {} | **{:.1}** | {:.1} | {:.1} | {:.1} | {:.1} |",
                bench.label,
                bench.timing.avg_ms,
                bench.timing.best_ms,
                bench.timing.worst_ms,
                bench.timing.median_ms,
                bench.timing.p95_ms,
            );

            if has_ru {
                if let Some(ru) = &bench.resource_usage {
                    row.push_str(&format!(
                        " {} | {} |",
                        format_cpu_total_ms(ru.cpu_total_ms),
                        format_peak_memory(ru.peak_memory_kb),
                    ));
                } else {
                    row.push_str(" — | — |");
                }
            }

            output.push_str(&row);
            output.push('\n');
        }

        output.push_str(&format!(
            "\n*{} iterations · {} warmup · avg is primary metric*\n",
            platform.iterations, platform.warmup
        ));
    }

    output
}

/// Enrich an offline report with BrowserStack session metrics.
pub fn enrich_with_browserstack(
    report: &mut SummarizeReport,
    build_summary: &crate::browserstack::BuildSummary,
) {
    for platform in &mut report.platforms {
        for session in &build_summary.sessions {
            if !session_matches_platform_row(platform, session) {
                continue;
            }

            if !session.os.is_empty() {
                platform.device.os = session.os.clone();
                platform.device.os_version = session.os_version.clone();
                if platform.device.name == "unknown" {
                    platform.device.name = session.device.clone();
                }
            }

            // Enrich benchmarks with performance metrics
            if let Some(perf) = &session.performance {
                for bench in &mut platform.benchmarks {
                    let peak_memory_kb = perf
                        .memory
                        .as_ref()
                        .map(|memory| (memory.peak_mb * 1024.0).round() as u64);
                    if peak_memory_kb.is_none() {
                        continue;
                    }
                    let resource_usage = bench.resource_usage.get_or_insert(ResourceUsage {
                        cpu_total_ms: None,
                        peak_memory_kb: None,
                        total_pss_kb: None,
                        private_dirty_kb: None,
                        native_heap_kb: None,
                        java_heap_kb: None,
                    });
                    resource_usage.peak_memory_kb = peak_memory_kb;
                }
            }
        }
    }
}

/// Render the report as JSON.
pub fn render_json(report: &SummarizeReport) -> Result<String> {
    serde_json::to_string_pretty(report).context("Failed to serialize report as JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn sample_summary_json() -> serde_json::Value {
        json!({
            "summary": {
                "generated_at": "2026-02-26T12:00:00Z",
                "target": "ios",
                "function": "bench_nullifier_proving_only",
                "iterations": 30,
                "warmup": 5,
                "devices": ["iPhone 14-16.0"],
                "device_summaries": [{
                    "device": "iPhone 14-16.0",
                    "benchmarks": [{
                        "function": "bench_nullifier_proving_only",
                        "samples": 30,
                        "mean_ns": 1204500000_u64,
                        "median_ns": 1198000000_u64,
                        "p95_ns": 1290000000_u64,
                        "min_ns": 1180200000_u64,
                        "max_ns": 1298100000_u64
                    }]
                }]
            }
        })
    }

    fn sample_bench_report_json() -> serde_json::Value {
        json!({
            "spec": {
                "name": "bench_nullifier_proving_only",
                "iterations": 3,
                "warmup": 1
            },
            "samples": [
                { "duration_ns": 1000_u64 },
                { "duration_ns": 2000_u64 },
                { "duration_ns": 3000_u64 }
            ]
        })
    }

    #[test]
    fn test_parse_summary_json() {
        let json = sample_summary_json();
        let report = parse_summary_value(&json).unwrap();
        assert_eq!(report.platforms.len(), 1);
        let p = &report.platforms[0];
        assert_eq!(p.platform, "ios");
        assert_eq!(p.iterations, 30);
        assert_eq!(p.warmup, 5);
        assert_eq!(p.benchmarks.len(), 1);
        let b = &p.benchmarks[0];
        assert!((b.timing.avg_ms - 1204.5).abs() < 0.1);
        assert!((b.timing.best_ms - 1180.2).abs() < 0.1);
        assert!((b.timing.worst_ms - 1298.1).abs() < 0.1);
    }

    #[test]
    fn test_parse_device_string_ios() {
        let d = parse_device_string("iPhone 14-16.0");
        assert_eq!(d.name, "iPhone 14");
        assert_eq!(d.os, "iOS");
        assert_eq!(d.os_version, "16.0");
    }

    #[test]
    fn test_parse_device_string_android() {
        let d = parse_device_string("Google Pixel 6-12.0");
        assert_eq!(d.name, "Google Pixel 6");
        assert_eq!(d.os, "Android");
        assert_eq!(d.os_version, "12.0");
    }

    #[test]
    fn test_humanize_benchmark_name() {
        assert_eq!(
            humanize_benchmark_name("bench_nullifier_proving_only"),
            "nullifier-proving-only"
        );
        assert_eq!(
            humanize_benchmark_name("bench_query_proof_generation"),
            "query-proof-generation"
        );
        // Qualified names use the last segment
        assert_eq!(
            humanize_benchmark_name("zk_mobile_bench::bench_fibonacci"),
            "fibonacci"
        );
        // No bench_ prefix is fine
        assert_eq!(humanize_benchmark_name("my_func"), "my-func");
    }

    #[test]
    fn test_load_results_dir() {
        let dir = tempfile::tempdir().unwrap();
        let json = sample_summary_json();
        std::fs::write(
            dir.path().join("test.json"),
            serde_json::to_string(&json).unwrap(),
        )
        .unwrap();

        let report = load_results_dir(dir.path()).unwrap();
        assert_eq!(report.platforms.len(), 1);
        assert_eq!(report.platforms[0].platform, "ios");
    }

    #[test]
    fn test_load_results_dir_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_results_dir(dir.path()).is_err());
    }

    #[test]
    fn test_parse_summary_value_handles_merged_ci_root() {
        let report = parse_summary_value(&json!({
            "targets": {
                "ios": {
                    "summary": sample_summary_json()["summary"].clone(),
                    "functions": {
                        "bench_nullifier_proving_only": sample_summary_json()
                    }
                }
            },
            "ci": {
                "metadata": {
                    "requested_by": "codex",
                    "request_command": "cargo mobench ci run",
                    "mobench_version": "0.1.0"
                },
                "outputs": {
                    "summary_json": "summary.json",
                    "summary_md": "summary.md",
                    "results_csv": "results.csv"
                }
            }
        }))
        .unwrap();

        assert_eq!(report.platforms.len(), 1);
        assert_eq!(report.platforms[0].platform, "ios");
        assert_eq!(report.platforms[0].benchmarks.len(), 1);
    }

    #[test]
    fn test_load_results_dir_recurses_into_target_function_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let ios_dir = dir.path().join("ios").join("bench_nullifier_proving_only");
        std::fs::create_dir_all(&ios_dir).unwrap();
        std::fs::write(
            ios_dir.join("summary.json"),
            serde_json::to_string(&sample_summary_json()).unwrap(),
        )
        .unwrap();

        let report = load_results_dir(dir.path()).unwrap();
        assert_eq!(report.platforms.len(), 1);
        assert_eq!(report.platforms[0].platform, "ios");
        assert_eq!(
            report.platforms[0].benchmarks[0].name,
            "bench_nullifier_proving_only"
        );
    }

    #[test]
    fn test_load_results_dir_prefers_canonical_target_summary_fixture() {
        let fixture_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ci-artifact-root");

        let report = load_results_dir(&fixture_dir).unwrap();

        assert_eq!(report.platforms.len(), 1);
        assert_eq!(report.platforms[0].platform, "android");
        assert_eq!(report.platforms[0].device.name, "Google Pixel 8");
        assert_eq!(report.platforms[0].benchmarks.len(), 2);
        assert_eq!(
            report.platforms[0]
                .benchmarks
                .iter()
                .map(|bench| bench.name.as_str())
                .collect::<Vec<_>>(),
            vec!["bench_alpha", "bench_beta"]
        );
    }

    #[test]
    fn test_load_results_dir_backfills_resource_usage_from_nested_summaries() {
        let fixture_dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ci-artifact-root");

        let report = load_results_dir(&fixture_dir).unwrap();
        let alpha = report.platforms[0]
            .benchmarks
            .iter()
            .find(|bench| bench.name == "bench_alpha")
            .expect("alpha benchmark");
        let beta = report.platforms[0]
            .benchmarks
            .iter()
            .find(|bench| bench.name == "bench_beta")
            .expect("beta benchmark");

        assert_eq!(
            alpha
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.cpu_total_ms),
            Some(111)
        );
        assert_eq!(
            alpha
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.peak_memory_kb),
            Some(222222)
        );
        assert_eq!(
            beta.resource_usage
                .as_ref()
                .and_then(|usage| usage.cpu_total_ms),
            Some(333)
        );
        assert_eq!(
            beta.resource_usage
                .as_ref()
                .and_then(|usage| usage.peak_memory_kb),
            Some(444444)
        );
    }

    #[test]
    fn test_load_results_dir_preserves_summary_peak_memory_when_raw_results_add_cpu() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("summary.json"),
            serde_json::to_string(&json!({
                "summary": {
                    "generated_at": "2026-03-25T12:00:00Z",
                    "generated_at_unix": 1742904000,
                    "target": "ios",
                    "function": "bench_nullifier_proving_only",
                    "iterations": 3,
                    "warmup": 1,
                    "devices": ["iPhone 15-17.0"],
                    "device_summaries": [{
                        "device": "iPhone 15-17.0",
                        "benchmarks": [{
                            "function": "bench_nullifier_proving_only",
                            "samples": 3,
                            "mean_ns": 125000000_u64,
                            "median_ns": 125000000_u64,
                            "p95_ns": 130000000_u64,
                            "min_ns": 120000000_u64,
                            "max_ns": 130000000_u64,
                            "resource_usage": {
                                "peak_memory_kb": 249416
                            }
                        }]
                    }]
                },
                "benchmark_results": {
                    "iPhone 15-17.0": [{
                        "function": "bench_nullifier_proving_only",
                        "mean_ns": 125000000_u64,
                        "samples": [
                            120000000_u64,
                            130000000_u64
                        ],
                        "resources": {
                            "elapsed_cpu_ms": 482
                        }
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let report = load_results_dir(dir.path()).unwrap();
        let benchmark = &report.platforms[0].benchmarks[0];

        assert_eq!(
            benchmark
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.cpu_total_ms),
            Some(482)
        );
        assert_eq!(
            benchmark
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.peak_memory_kb),
            Some(249416)
        );
    }

    #[test]
    fn test_load_results_dir_falls_back_to_raw_bench_report() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("bench-report.json"),
            serde_json::to_string(&sample_bench_report_json()).unwrap(),
        )
        .unwrap();

        let report = load_results_dir(dir.path()).unwrap();
        assert_eq!(report.platforms.len(), 1);
        assert_eq!(report.platforms[0].benchmarks.len(), 1);
        assert_eq!(
            report.platforms[0].benchmarks[0].name,
            "bench_nullifier_proving_only"
        );
    }

    #[test]
    fn test_render_table_output() {
        let report = SummarizeReport {
            platforms: vec![PlatformReport {
                platform: "ios".to_string(),
                device: DeviceInfo {
                    name: "iPhone 14".to_string(),
                    os: "iOS".to_string(),
                    os_version: "16.0".to_string(),
                    chipset: Some("A15 Bionic".to_string()),
                    ram_gb: Some(6.0),
                },
                benchmarks: vec![BenchmarkResult {
                    name: "bench_nullifier_proving_only".to_string(),
                    label: "nullifier-proving-only".to_string(),
                    timing: TimingStats {
                        avg_ms: 1204.5,
                        median_ms: 1198.0,
                        best_ms: 1180.2,
                        worst_ms: 1298.1,
                        p95_ms: 1290.0,
                        std_dev_ms: Some(35.2),
                    },
                    resource_usage: Some(ResourceUsage {
                        cpu_total_ms: Some(482),
                        peak_memory_kb: Some(654321),
                        total_pss_kb: Some(654321),
                        private_dirty_kb: Some(321000),
                        native_heap_kb: Some(120000),
                        java_heap_kb: Some(45000),
                    }),
                }],
                iterations: 30,
                warmup: 5,
            }],
        };
        let output = render_table(&report);
        assert!(output.contains("iPhone 14"));
        assert!(output.contains("1204.5"));
        assert!(output.contains("A15 Bionic"));
        assert!(output.contains("6 GB RAM"));
    }

    #[test]
    fn test_render_markdown_output() {
        let report = SummarizeReport {
            platforms: vec![PlatformReport {
                platform: "ios".to_string(),
                device: DeviceInfo {
                    name: "iPhone 14".to_string(),
                    os: "iOS".to_string(),
                    os_version: "16.0".to_string(),
                    chipset: None,
                    ram_gb: None,
                },
                benchmarks: vec![BenchmarkResult {
                    name: "bench_nullifier_proving_only".to_string(),
                    label: "nullifier-proving-only".to_string(),
                    timing: TimingStats {
                        avg_ms: 1204.5,
                        median_ms: 1198.0,
                        best_ms: 1180.2,
                        worst_ms: 1298.1,
                        p95_ms: 1290.0,
                        std_dev_ms: Some(35.2),
                    },
                    resource_usage: None,
                }],
                iterations: 30,
                warmup: 5,
            }],
        };
        let output = render_markdown(&report);
        assert!(output.contains("### IOS"));
        assert!(output.contains("**1204.5**"));
        assert!(output.contains("| Benchmark |"));
    }

    #[test]
    fn test_render_markdown_uses_cpu_total_and_peak_memory_columns() {
        let report = parse_summary_value(&json!({
            "summary": {
                "generated_at": "2026-02-26T12:00:00Z",
                "target": "android",
                "function": "bench_nullifier_proving_only",
                "iterations": 30,
                "warmup": 5,
                "devices": ["Google Pixel 8-14.0"],
                "device_summaries": [{
                    "device": "Google Pixel 8-14.0",
                    "benchmarks": [{
                        "function": "bench_nullifier_proving_only",
                        "samples": 30,
                        "mean_ns": 1204500000_u64,
                        "median_ns": 1198000000_u64,
                        "p95_ns": 1290000000_u64,
                        "min_ns": 1180200000_u64,
                        "max_ns": 1298100000_u64,
                        "resource_usage": {
                            "cpu_total_ms": 482,
                            "peak_memory_kb": 654321,
                            "total_pss_kb": 654321
                        }
                    }]
                }]
            }
        }))
        .unwrap();

        let output = render_markdown(&report);

        assert!(output.contains("CPU total (ms)"));
        assert!(output.contains("Peak memory"));
        assert!(output.contains("| 482 |"));
        assert!(output.contains("638.99 MB"));
        assert!(!output.contains("CPU %"));
        assert!(!output.contains("RAM MB"));
    }

    #[test]
    fn test_render_table_uses_cpu_total_and_peak_memory_columns() {
        let report = parse_summary_value(&json!({
            "summary": {
                "generated_at": "2026-02-26T12:00:00Z",
                "target": "android",
                "function": "bench_nullifier_proving_only",
                "iterations": 30,
                "warmup": 5,
                "devices": ["Google Pixel 8-14.0"],
                "device_summaries": [{
                    "device": "Google Pixel 8-14.0",
                    "benchmarks": [{
                        "function": "bench_nullifier_proving_only",
                        "samples": 30,
                        "mean_ns": 1204500000_u64,
                        "median_ns": 1198000000_u64,
                        "p95_ns": 1290000000_u64,
                        "min_ns": 1180200000_u64,
                        "max_ns": 1298100000_u64,
                        "resource_usage": {
                            "cpu_total_ms": 482,
                            "peak_memory_kb": 654321,
                            "total_pss_kb": 654321
                        }
                    }]
                }]
            }
        }))
        .unwrap();

        let output = render_table(&report);

        assert!(output.contains("CPU total (ms)"));
        assert!(output.contains("Peak memory"));
        assert!(output.contains("482"));
        assert!(output.contains("638.99 MB"));
        assert!(!output.contains("CPU %"));
        assert!(!output.contains("RAM MB"));
    }

    #[test]
    fn test_render_json_output() {
        let report = SummarizeReport {
            platforms: vec![PlatformReport {
                platform: "ios".to_string(),
                device: DeviceInfo {
                    name: "iPhone 14".to_string(),
                    os: "iOS".to_string(),
                    os_version: "16.0".to_string(),
                    chipset: None,
                    ram_gb: None,
                },
                benchmarks: vec![],
                iterations: 30,
                warmup: 5,
            }],
        };
        let json_str = render_json(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["platforms"][0]["platform"], "ios");
    }

    #[test]
    fn test_enrich_with_browserstack_matches_session_to_same_device_row() {
        let mut report = SummarizeReport {
            platforms: vec![
                PlatformReport {
                    platform: "android".to_string(),
                    device: DeviceInfo {
                        name: "Google Pixel 6".to_string(),
                        os: "Android".to_string(),
                        os_version: "14".to_string(),
                        chipset: None,
                        ram_gb: None,
                    },
                    benchmarks: vec![BenchmarkResult {
                        name: "bench_alpha".to_string(),
                        label: "alpha".to_string(),
                        timing: TimingStats {
                            avg_ms: 10.0,
                            median_ms: 10.0,
                            best_ms: 10.0,
                            worst_ms: 10.0,
                            p95_ms: 10.0,
                            std_dev_ms: None,
                        },
                        resource_usage: None,
                    }],
                    iterations: 5,
                    warmup: 1,
                },
                PlatformReport {
                    platform: "android".to_string(),
                    device: DeviceInfo {
                        name: "Samsung Galaxy S24".to_string(),
                        os: "Android".to_string(),
                        os_version: "14".to_string(),
                        chipset: None,
                        ram_gb: None,
                    },
                    benchmarks: vec![BenchmarkResult {
                        name: "bench_alpha".to_string(),
                        label: "alpha".to_string(),
                        timing: TimingStats {
                            avg_ms: 10.0,
                            median_ms: 10.0,
                            best_ms: 10.0,
                            worst_ms: 10.0,
                            p95_ms: 10.0,
                            std_dev_ms: None,
                        },
                        resource_usage: None,
                    }],
                    iterations: 5,
                    warmup: 1,
                },
            ],
        };

        let build_summary = crate::browserstack::BuildSummary {
            build_id: "build-123".to_string(),
            status: "done".to_string(),
            sessions: vec![
                crate::browserstack::SessionSummary {
                    session_id: "session-pixel".to_string(),
                    device: "Google Pixel 6".to_string(),
                    os: "android".to_string(),
                    os_version: "14".to_string(),
                    duration_secs: Some(10),
                    performance: Some(crate::browserstack::PerformanceMetrics {
                        memory: Some(crate::browserstack::AggregateMemoryMetrics {
                            peak_mb: 100.0,
                            average_mb: 90.0,
                            min_mb: 80.0,
                        }),
                        cpu: None,
                        sample_count: 1,
                        snapshots: vec![],
                    }),
                },
                crate::browserstack::SessionSummary {
                    session_id: "session-samsung".to_string(),
                    device: "Samsung Galaxy S24".to_string(),
                    os: "android".to_string(),
                    os_version: "14".to_string(),
                    duration_secs: Some(10),
                    performance: Some(crate::browserstack::PerformanceMetrics {
                        memory: Some(crate::browserstack::AggregateMemoryMetrics {
                            peak_mb: 200.0,
                            average_mb: 190.0,
                            min_mb: 180.0,
                        }),
                        cpu: None,
                        sample_count: 1,
                        snapshots: vec![],
                    }),
                },
            ],
        };

        enrich_with_browserstack(&mut report, &build_summary);

        let pixel_memory = report.platforms[0].benchmarks[0]
            .resource_usage
            .as_ref()
            .and_then(|usage| usage.peak_memory_kb);
        let samsung_memory = report.platforms[1].benchmarks[0]
            .resource_usage
            .as_ref()
            .and_then(|usage| usage.peak_memory_kb);

        assert_eq!(pixel_memory, Some(102_400));
        assert_eq!(samsung_memory, Some(204_800));
    }
}
