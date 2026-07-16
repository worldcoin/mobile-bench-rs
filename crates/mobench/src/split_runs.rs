use crate::{
    BenchmarkResourceUsage, BenchmarkStats, CiMergeSplitRunsArgs, DeviceSummary, MobileTarget,
    SummaryReport, compute_sample_stats, ensure_parent_dir, json_value_to_u64, median_u64,
    render_csv_summary, render_markdown_summary, summarize::device_names_match,
    summary_report_from_value, write_file,
};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub(crate) fn cmd_ci_merge_split_runs(args: CiMergeSplitRunsArgs, dry_run: bool) -> Result<()> {
    let merged = merge_split_run_summaries(
        &args.samples_dir,
        &args.function,
        &args.device,
        args.iterations,
        args.warmup,
    )?;

    fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("creating output directory {}", args.output_dir.display()))?;
    let summary_json = args.output_dir.join("summary.json");
    let summary_md = args.output_dir.join("summary.md");
    let results_csv = args.output_dir.join("results.csv");

    if dry_run {
        println!("Would merge split-run CI outputs:");
        println!(" - {}", summary_json.display());
        println!(" - {}", summary_md.display());
        println!(" - {}", results_csv.display());
        return Ok(());
    }

    let output = json!({
        "summary": merged.summary,
        "benchmark_results": {
            merged.device.clone(): [merged.benchmark]
        },
        "ci": {
            "metadata": {
                "request_command": "cargo mobench ci merge-split-runs",
                "mobench_version": env!("CARGO_PKG_VERSION")
            },
            "outputs": {
                "summary_json": "summary.json",
                "summary_md": "summary.md",
                "results_csv": "results.csv"
            }
        }
    });

    ensure_parent_dir(&summary_json)?;
    write_file(
        &summary_json,
        serde_json::to_string_pretty(&output)?.as_bytes(),
    )?;
    write_file(
        &summary_md,
        render_markdown_summary(&merged.summary).as_bytes(),
    )?;
    write_file(&results_csv, render_csv_summary(&merged.summary).as_bytes())?;

    println!("Merged split-run CI outputs:");
    println!(" - {}", summary_json.display());
    println!(" - {}", summary_md.display());
    println!(" - {}", results_csv.display());

    Ok(())
}

#[derive(Debug)]
struct MergedSplitRun {
    summary: SummaryReport,
    device: String,
    benchmark: Value,
}

#[derive(Debug)]
struct SplitSample {
    path: PathBuf,
    target: MobileTarget,
    benchmark: Value,
    samples: Vec<Value>,
}

fn merge_split_run_summaries(
    samples_dir: &Path,
    function: &str,
    device: &str,
    iterations: u32,
    warmup: u32,
) -> Result<MergedSplitRun> {
    let samples = load_split_samples(samples_dir, function, device)?;
    let first = samples.first().ok_or_else(|| {
        anyhow!(
            "no sample summary.json files found under {}",
            samples_dir.display()
        )
    })?;

    for sample in &samples {
        if sample.target != first.target {
            bail!(
                "{} target {} does not match first sample target {}",
                sample.path.display(),
                sample.target.as_str(),
                first.target.as_str()
            );
        }
    }

    let mut sample_values = Vec::new();
    for sample in &samples {
        if sample.samples.len() != 1 {
            bail!(
                "{} expected exactly one measured sample, got {}",
                sample.path.display(),
                sample.samples.len()
            );
        }
        sample_values.extend(sample.samples.iter().cloned());
    }

    if sample_values.len() != iterations as usize {
        bail!(
            "expected {iterations} measured samples, got {}",
            sample_values.len()
        );
    }

    let samples_ns = sample_values
        .iter()
        .map(sample_duration_ns)
        .collect::<Result<Vec<_>>>()?;
    let stats = compute_sample_stats(&samples_ns)
        .ok_or_else(|| anyhow!("cannot merge split runs without measured samples"))?;
    let resource_usage = merge_resource_usage(&samples, &sample_values);

    let mut benchmark = samples
        .first()
        .map(|sample| sample.benchmark.clone())
        .unwrap_or_else(|| json!({}));
    set_object_value(&mut benchmark, "function", json!(function));
    set_object_value(&mut benchmark, "samples", Value::Array(sample_values));
    set_object_value(&mut benchmark, "samples_ns", json!(samples_ns));
    set_object_value(&mut benchmark, "min_ns", json!(stats.min_ns));
    set_object_value(&mut benchmark, "max_ns", json!(stats.max_ns));
    set_object_value(&mut benchmark, "mean_ns", json!(stats.mean_ns));
    set_object_value(&mut benchmark, "median_ns", json!(stats.median_ns));
    set_object_value(&mut benchmark, "p95_ns", json!(stats.p95_ns));
    set_object_value(
        &mut benchmark,
        "stats",
        json!({
            "avg_ns": stats.mean_ns,
            "mean_ns": stats.mean_ns,
            "median_ns": stats.median_ns,
            "min_ns": stats.min_ns,
            "max_ns": stats.max_ns,
            "p95_ns": stats.p95_ns
        }),
    );
    if let Some(resource_usage) = &resource_usage {
        set_object_value(
            &mut benchmark,
            "resources",
            serde_json::to_value(resource_usage)?,
        );
        set_object_value(
            &mut benchmark,
            "resource_usage",
            serde_json::to_value(resource_usage)?,
        );
    }
    merge_benchmark_spec(&mut benchmark, function, iterations, warmup);

    let summary = SummaryReport {
        generated_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "unknown".to_string()),
        generated_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("generating timestamp")?
            .as_secs(),
        target: first.target,
        function: function.to_string(),
        iterations,
        warmup,
        devices: vec![device.to_string()],
        device_summaries: vec![DeviceSummary {
            device: device.to_string(),
            benchmarks: vec![BenchmarkStats {
                function: function.to_string(),
                samples: iterations as usize,
                mean_ns: Some(stats.mean_ns),
                median_ns: Some(stats.median_ns),
                p95_ns: Some(stats.p95_ns),
                min_ns: Some(stats.min_ns),
                max_ns: Some(stats.max_ns),
                resource_usage,
                failure: None,
            }],
        }],
    };

    Ok(MergedSplitRun {
        summary,
        device: device.to_string(),
        benchmark,
    })
}

fn load_split_samples(
    samples_dir: &Path,
    function: &str,
    device: &str,
) -> Result<Vec<SplitSample>> {
    let mut paths = fs::read_dir(samples_dir)
        .with_context(|| format!("reading samples directory {}", samples_dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("iterating samples directory {}", samples_dir.display()))?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("sample-"))
                && path.join("summary.json").exists()
        })
        .map(|path| path.join("summary.json"))
        .collect::<Vec<_>>();
    paths.sort();

    if paths.is_empty() {
        bail!(
            "no sample summary.json files found under {}",
            samples_dir.display()
        );
    }

    paths
        .into_iter()
        .map(|path| load_split_sample(&path, function, device))
        .collect()
}

fn load_split_sample(
    path: &Path,
    expected_function: &str,
    expected_device: &str,
) -> Result<SplitSample> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading split-run summary {}", path.display()))?;
    let value: Value = serde_json::from_str(&content)
        .with_context(|| format!("parsing split-run summary {}", path.display()))?;
    let summary = summary_report_from_value(&value)
        .with_context(|| format!("reading summary section {}", path.display()))?;

    let device_summary = exactly_one(&summary.device_summaries, "device summary", path)?;
    if !device_names_match(&device_summary.device, expected_device) {
        bail!(
            "{} device `{}` does not match requested `{}`",
            path.display(),
            device_summary.device,
            expected_device
        );
    }
    let summary_benchmark = exactly_one(&device_summary.benchmarks, "benchmark", path)?;
    if summary_benchmark.function != expected_function {
        bail!(
            "{} benchmark `{}` does not match requested `{}`",
            path.display(),
            summary_benchmark.function,
            expected_function
        );
    }

    let benchmark = raw_benchmark_for_device(&value, expected_device, path)?
        .unwrap_or_else(|| summary_benchmark_to_value(summary_benchmark));
    let raw_function = benchmark
        .get("function")
        .and_then(|value| value.as_str())
        .unwrap_or(&summary_benchmark.function);
    if raw_function != expected_function {
        bail!(
            "{} raw benchmark `{}` does not match requested `{}`",
            path.display(),
            raw_function,
            expected_function
        );
    }

    let samples = extract_sample_values(&benchmark)
        .or_else(|| summary_single_sample_value(summary_benchmark))
        .with_context(|| format!("extracting measured sample from {}", path.display()))?;

    Ok(SplitSample {
        path: path.to_path_buf(),
        target: summary.target,
        benchmark,
        samples,
    })
}

fn exactly_one<'a, T>(items: &'a [T], label: &str, path: &Path) -> Result<&'a T> {
    if items.len() != 1 {
        bail!(
            "{} expected exactly one {}, got {}",
            path.display(),
            label,
            items.len()
        );
    }
    Ok(&items[0])
}

fn raw_benchmark_for_device(
    value: &Value,
    expected_device: &str,
    path: &Path,
) -> Result<Option<Value>> {
    let Some(results) = value
        .get("benchmark_results")
        .and_then(|value| value.as_object())
    else {
        return Ok(None);
    };
    if results.len() != 1 {
        bail!(
            "{} expected exactly one device in benchmark_results, got {}",
            path.display(),
            results.len()
        );
    }
    let (device, benchmarks) = results.iter().next().expect("non-empty benchmark_results");
    if !device_names_match(device, expected_device) {
        bail!(
            "{} benchmark_results device `{}` does not match requested `{}`",
            path.display(),
            device,
            expected_device
        );
    }
    let benchmarks = benchmarks.as_array().ok_or_else(|| {
        anyhow!(
            "{} benchmark_results entry must be an array",
            path.display()
        )
    })?;
    if benchmarks.len() != 1 {
        bail!(
            "{} expected exactly one benchmark in benchmark_results, got {}",
            path.display(),
            benchmarks.len()
        );
    }
    Ok(Some(benchmarks[0].clone()))
}

fn summary_benchmark_to_value(benchmark: &BenchmarkStats) -> Value {
    json!({
        "function": benchmark.function,
        "samples": benchmark.samples,
        "mean_ns": benchmark.mean_ns,
        "median_ns": benchmark.median_ns,
        "p95_ns": benchmark.p95_ns,
        "min_ns": benchmark.min_ns,
        "max_ns": benchmark.max_ns,
        "resource_usage": benchmark.resource_usage,
    })
}

fn extract_sample_values(benchmark: &Value) -> Option<Vec<Value>> {
    let samples = benchmark
        .get("samples")
        .and_then(|value| value.as_array())?;
    let values = samples
        .iter()
        .filter_map(|sample| {
            if sample
                .get("duration_ns")
                .and_then(json_value_to_u64)
                .is_some()
            {
                Some(sample.clone())
            } else {
                json_value_to_u64(sample).map(|duration| json!({ "duration_ns": duration }))
            }
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn summary_single_sample_value(benchmark: &BenchmarkStats) -> Option<Vec<Value>> {
    if benchmark.samples != 1 {
        return None;
    }
    let duration = benchmark
        .mean_ns
        .or(benchmark.median_ns)
        .or(benchmark.min_ns)
        .or(benchmark.max_ns)?;
    Some(vec![json!({ "duration_ns": duration })])
}

fn sample_duration_ns(sample: &Value) -> Result<u64> {
    sample
        .get("duration_ns")
        .and_then(json_value_to_u64)
        .or_else(|| json_value_to_u64(sample))
        .ok_or_else(|| anyhow!("sample is missing duration_ns"))
}

fn merge_resource_usage(
    samples: &[SplitSample],
    sample_values: &[Value],
) -> Option<BenchmarkResourceUsage> {
    let sample_cpu_ms = collect_from_samples(sample_values, "cpu_time_ms");
    let sample_peak_memory_kb = collect_from_samples(sample_values, "peak_memory_kb");
    let sample_process_peak_memory_kb =
        collect_from_samples(sample_values, "process_peak_memory_kb");

    let resource_values = samples
        .iter()
        .filter_map(|sample| {
            sample
                .benchmark
                .get("resource_usage")
                .or_else(|| sample.benchmark.get("resources"))
        })
        .collect::<Vec<_>>();

    let usage = BenchmarkResourceUsage {
        cpu_total_ms: if sample_cpu_ms.is_empty() {
            sum_resource_values(&resource_values, &["cpu_total_ms", "elapsed_cpu_ms"])
        } else {
            Some(sample_cpu_ms.iter().sum())
        },
        cpu_median_ms: median_u64(&sample_cpu_ms)
            .or_else(|| median_resource_values(&resource_values, &["cpu_median_ms"])),
        peak_memory_kb: max_sample_or_resource(
            &sample_peak_memory_kb,
            &resource_values,
            &["peak_memory_kb"],
        ),
        peak_memory_growth_kb: max_resource_value(&resource_values, &["peak_memory_growth_kb"]),
        process_peak_memory_kb: max_sample_or_resource(
            &sample_process_peak_memory_kb,
            &resource_values,
            &["process_peak_memory_kb"],
        ),
        total_pss_kb: max_resource_value(&resource_values, &["total_pss_kb"]),
        private_dirty_kb: max_resource_value(&resource_values, &["private_dirty_kb"]),
        native_heap_kb: max_resource_value(&resource_values, &["native_heap_kb"]),
        java_heap_kb: max_resource_value(&resource_values, &["java_heap_kb"]),
    };

    (!usage.is_empty()).then_some(usage)
}

fn collect_from_samples(samples: &[Value], key: &str) -> Vec<u64> {
    samples
        .iter()
        .filter_map(|sample| sample.get(key))
        .filter_map(json_value_to_u64)
        .collect()
}

fn resource_numbers(resources: &[&Value], keys: &[&str]) -> Vec<u64> {
    resources
        .iter()
        .filter_map(|resource| keys.iter().find_map(|key| resource.get(*key)))
        .filter_map(json_value_to_u64)
        .collect()
}

fn sum_resource_values(resources: &[&Value], keys: &[&str]) -> Option<u64> {
    let values = resource_numbers(resources, keys);
    (!values.is_empty()).then(|| values.iter().sum())
}

fn median_resource_values(resources: &[&Value], keys: &[&str]) -> Option<u64> {
    median_u64(&resource_numbers(resources, keys))
}

fn max_resource_value(resources: &[&Value], keys: &[&str]) -> Option<u64> {
    resource_numbers(resources, keys).into_iter().max()
}

fn max_sample_or_resource(samples: &[u64], resources: &[&Value], keys: &[&str]) -> Option<u64> {
    samples
        .iter()
        .copied()
        .max()
        .or_else(|| max_resource_value(resources, keys))
}

fn set_object_value(value: &mut Value, key: &str, item: Value) {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .expect("value was converted to object")
        .insert(key.to_string(), item);
}

fn merge_benchmark_spec(benchmark: &mut Value, function: &str, iterations: u32, warmup: u32) {
    let mut spec = benchmark
        .get("spec")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    spec.insert("name".to_string(), json!(function));
    spec.insert("iterations".to_string(), json!(iterations));
    spec.insert("warmup".to_string(), json!(warmup));
    set_object_value(benchmark, "spec", Value::Object(spec));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_sample(dir: &Path, name: &str, function: &str, device: &str, duration_ns: u64) {
        let sample_dir = dir.join(name);
        fs::create_dir_all(&sample_dir).unwrap();
        let summary = json!({
            "summary": {
                "generated_at": "2026-07-05T12:00:00Z",
                "generated_at_unix": 1783252800_u64,
                "target": "android",
                "function": function,
                "iterations": 1,
                "warmup": 1,
                "devices": [device],
                "device_summaries": [{
                    "device": device,
                    "benchmarks": [{
                        "function": function,
                        "samples": 1,
                        "mean_ns": duration_ns,
                        "median_ns": duration_ns,
                        "p95_ns": duration_ns,
                        "min_ns": duration_ns,
                        "max_ns": duration_ns,
                        "resource_usage": {
                            "cpu_total_ms": duration_ns / 1_000_000,
                            "cpu_median_ms": duration_ns / 1_000_000,
                            "peak_memory_kb": 1024
                        }
                    }]
                }]
            },
            "benchmark_results": {
                device: [{
                    "function": function,
                    "samples": [{
                        "duration_ns": duration_ns,
                        "cpu_time_ms": duration_ns / 1_000_000,
                        "peak_memory_kb": 1024,
                        "process_peak_memory_kb": 4096
                    }]
                }]
            }
        });
        fs::write(
            sample_dir.join("summary.json"),
            serde_json::to_string_pretty(&summary).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn merge_split_run_summaries_recomputes_stats() {
        let temp = tempfile::tempdir().unwrap();
        let samples_dir = temp.path().join("split");
        let output_dir = temp.path().join("merged");
        fs::create_dir_all(&samples_dir).unwrap();
        let function = "bench_mobile::bench_passport_complete_age_check_prove";
        let device = "Samsung Galaxy M32-11.0";
        write_sample(&samples_dir, "sample-1", function, device, 100_000_000);
        write_sample(&samples_dir, "sample-2", function, device, 200_000_000);
        write_sample(&samples_dir, "sample-3", function, device, 300_000_000);

        cmd_ci_merge_split_runs(
            CiMergeSplitRunsArgs {
                samples_dir,
                output_dir: output_dir.clone(),
                function: function.to_string(),
                device: device.to_string(),
                iterations: 3,
                warmup: 1,
            },
            false,
        )
        .unwrap();

        assert!(output_dir.join("summary.json").exists());
        assert!(output_dir.join("summary.md").exists());
        assert!(output_dir.join("results.csv").exists());

        let summary: Value =
            serde_json::from_str(&fs::read_to_string(output_dir.join("summary.json")).unwrap())
                .unwrap();
        let benchmark = &summary["summary"]["device_summaries"][0]["benchmarks"][0];
        assert_eq!(benchmark["samples"], 3);
        assert_eq!(benchmark["mean_ns"], 200_000_000);
        assert_eq!(benchmark["median_ns"], 200_000_000);
        assert_eq!(benchmark["p95_ns"], 300_000_000);
        assert_eq!(benchmark["min_ns"], 100_000_000);
        assert_eq!(benchmark["max_ns"], 300_000_000);
        assert_eq!(benchmark["resource_usage"]["cpu_total_ms"], 600);
        assert_eq!(
            summary["benchmark_results"][device][0]["samples_ns"],
            json!([100_000_000, 200_000_000, 300_000_000])
        );
        assert_eq!(
            summary["benchmark_results"][device][0]["stats"]["avg_ns"],
            200_000_000
        );
        assert!(
            fs::read_to_string(output_dir.join("summary.md"))
                .unwrap()
                .contains("3 / 1")
        );
        assert!(
            fs::read_to_string(output_dir.join("results.csv"))
                .unwrap()
                .contains("device,function,samples")
        );
    }

    #[test]
    fn merge_split_run_summaries_rejects_mismatched_function() {
        let temp = tempfile::tempdir().unwrap();
        let samples_dir = temp.path().join("split");
        fs::create_dir_all(&samples_dir).unwrap();
        let device = "Samsung Galaxy M32-11.0";
        write_sample(&samples_dir, "sample-1", "wrong::function", device, 100);

        let err = merge_split_run_summaries(&samples_dir, "expected::function", device, 1, 0)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match requested"));
    }

    #[test]
    fn merge_split_run_summaries_rejects_mismatched_device() {
        let temp = tempfile::tempdir().unwrap();
        let samples_dir = temp.path().join("split");
        fs::create_dir_all(&samples_dir).unwrap();
        let function = "bench_mobile::bench_passport_complete_age_check_prove";
        write_sample(
            &samples_dir,
            "sample-1",
            function,
            "Google Pixel 8-14.0",
            100,
        );

        let err =
            merge_split_run_summaries(&samples_dir, function, "Samsung Galaxy M32-11.0", 1, 0)
                .unwrap_err()
                .to_string();
        assert!(err.contains("does not match requested"));
    }

    #[test]
    fn merge_split_run_summaries_accepts_reported_device_without_os_suffix() {
        let temp = tempfile::tempdir().unwrap();
        let samples_dir = temp.path().join("split");
        fs::create_dir_all(&samples_dir).unwrap();
        let function = "bench_mobile::bench_passport_complete_age_check_prove";
        write_sample(
            &samples_dir,
            "sample-1",
            function,
            "Samsung Galaxy M32",
            100,
        );

        merge_split_run_summaries(&samples_dir, function, "Samsung Galaxy M32-11.0", 1, 0).unwrap();
    }
}
