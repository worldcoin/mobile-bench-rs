use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlotFunctionInput {
    pub function_name: String,
    pub function_label: String,
    pub target: String,
    pub iterations: u32,
    pub warmup: u32,
    pub devices: Vec<PlotDeviceSamples>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlotDeviceSamples {
    pub device_name: String,
    pub os_version: String,
    pub samples_ns: Vec<u64>,
}

#[test]
fn extract_function_plot_inputs_reads_fixture_samples() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ci-artifact-root");

    let plots = extract_function_plot_inputs_from_results_dir(&root)
        .expect("extract plot inputs");

    let alpha = plots
        .iter()
        .find(|plot| plot.function_name == "bench_alpha")
        .expect("bench_alpha plot");

    assert_eq!(alpha.devices.len(), 1);
    assert_eq!(alpha.devices[0].device_name, "Google Pixel 8");
    assert_eq!(
        alpha.devices[0].samples_ns,
        vec![95_000_000, 98_000_000, 100_000_000, 120_000_000, 123_000_000]
    );
}

#[test]
fn extract_function_plot_inputs_walks_nested_files_without_duplicates() {
    let root = tempfile::tempdir().expect("tempdir");
    let root_summary = root.path().join("summary.json");
    let nested_dir = root.path().join("android").join("bench_beta");
    fs::create_dir_all(&nested_dir).expect("create nested dir");

    write_json(
        &root_summary,
        serde_json::json!({
            "summary": {
                "generated_at": "2026-03-25T00:00:00Z",
                "generated_at_unix": 1_000_000_000_u64,
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
                        "mean_ns": 100_u64,
                        "median_ns": 100_u64,
                        "p95_ns": 100_u64,
                        "min_ns": 100_u64,
                        "max_ns": 100_u64
                    }]
                }]
            },
            "benchmark_results": {
                "Google Pixel 8-14.0": [{
                    "function": "bench_alpha",
                    "samples": [100_u64, 200_u64, 300_u64]
                }]
            }
        }),
    );

    write_json(
        &nested_dir.join("summary.json"),
        serde_json::json!({
            "summary": {
                "generated_at": "2026-03-25T00:00:00Z",
                "generated_at_unix": 1_000_000_001_u64,
                "target": "android",
                "function": "bench_beta",
                "iterations": 3,
                "warmup": 1,
                "devices": ["Google Pixel 8-14.0"],
                "device_summaries": [{
                    "device": "Google Pixel 8-14.0",
                    "benchmarks": [{
                        "function": "bench_beta",
                        "samples": 3,
                        "mean_ns": 400_u64,
                        "median_ns": 400_u64,
                        "p95_ns": 400_u64,
                        "min_ns": 400_u64,
                        "max_ns": 400_u64
                    }]
                }]
            },
            "benchmark_results": {
                "Google Pixel 8-14.0": [{
                    "function": "bench_alpha",
                    "samples": [100_u64, 200_u64, 300_u64]
                }, {
                    "function": "bench_beta",
                    "samples": [400_u64, 500_u64, 600_u64]
                }]
            }
        }),
    );

    let plots = extract_function_plot_inputs_from_results_dir(root.path())
        .expect("extract plot inputs");

    let alpha = plots
        .iter()
        .find(|plot| plot.function_name == "bench_alpha")
        .expect("bench_alpha plot");
    let beta = plots
        .iter()
        .find(|plot| plot.function_name == "bench_beta")
        .expect("bench_beta plot");

    assert_eq!(alpha.devices.len(), 1);
    assert_eq!(alpha.devices[0].samples_ns, vec![100, 200, 300]);
    assert_eq!(beta.devices.len(), 1);
    assert_eq!(beta.devices[0].samples_ns, vec![400, 500, 600]);
}

#[test]
fn extract_function_plot_inputs_preserves_duplicate_samples_from_a_single_payload() {
    let root = tempfile::tempdir().expect("tempdir");
    let root_summary = root.path().join("summary.json");
    let nested_dir = root.path().join("android").join("bench_alpha");
    fs::create_dir_all(&nested_dir).expect("create nested dir");

    let payload = serde_json::json!({
        "function": "bench_alpha",
        "samples": [100_u64, 100_u64, 200_u64]
    });

    write_json(
        &root_summary,
        serde_json::json!({
            "summary": {
                "generated_at": "2026-03-25T00:00:00Z",
                "generated_at_unix": 1_000_000_002_u64,
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
                        "mean_ns": 133_u64,
                        "median_ns": 100_u64,
                        "p95_ns": 200_u64,
                        "min_ns": 100_u64,
                        "max_ns": 200_u64
                    }]
                }]
            },
            "benchmark_results": {
                "Google Pixel 8-14.0": [payload]
            }
        }),
    );

    write_json(
        &nested_dir.join("summary.json"),
        serde_json::json!({
            "summary": {
                "generated_at": "2026-03-25T00:00:00Z",
                "generated_at_unix": 1_000_000_003_u64,
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
                        "mean_ns": 133_u64,
                        "median_ns": 100_u64,
                        "p95_ns": 200_u64,
                        "min_ns": 100_u64,
                        "max_ns": 200_u64
                    }]
                }]
            },
            "benchmark_results": {
                "Google Pixel 8-14.0": [payload]
            }
        }),
    );

    let plots = extract_function_plot_inputs_from_results_dir(root.path())
        .expect("extract plot inputs");

    let alpha = plots
        .iter()
        .find(|plot| plot.function_name == "bench_alpha")
        .expect("bench_alpha plot");

    assert_eq!(alpha.devices.len(), 1);
    assert_eq!(
        alpha.devices[0].samples_ns,
        vec![100, 100, 200]
    );
}

pub fn extract_function_plot_inputs_from_results_dir(dir: &Path) -> Result<Vec<PlotFunctionInput>> {
    let mut builders = BTreeMap::new();

    collect_from_results_dir(dir, &mut builders)?;

    let mut plots = builders
        .into_values()
        .map(PlotFunctionInputBuilder::finish)
        .collect::<Vec<_>>();
    plots.sort_by(|left, right| {
        left.function_name
            .cmp(&right.function_name)
            .then(left.target.cmp(&right.target))
    });
    Ok(plots)
}

#[derive(Debug, Default)]
struct PlotFunctionInputBuilder {
    function_name: String,
    function_label: String,
    target: String,
    iterations: u32,
    warmup: u32,
    devices: BTreeMap<(String, String), PlotDeviceSamplesBuilder>,
}

#[derive(Debug, Default)]
struct PlotDeviceSamplesBuilder {
    device_name: String,
    os_version: String,
    samples_ns: Vec<u64>,
    seen_payloads: BTreeSet<String>,
}

impl PlotFunctionInputBuilder {
    fn new(function_name: String, target: String) -> Self {
        Self {
            function_label: humanize_benchmark_name(&function_name),
            function_name,
            target,
            iterations: 0,
            warmup: 0,
            devices: BTreeMap::new(),
        }
    }

    fn set_run_metadata(&mut self, iterations: u32, warmup: u32) {
        if self.iterations == 0 {
            self.iterations = iterations;
        }
        if self.warmup == 0 {
            self.warmup = warmup;
        }
    }

    fn add_device_samples(
        &mut self,
        device_name: String,
        os_version: String,
        source_signature: String,
        samples_ns: Vec<u64>,
    ) {
        if samples_ns.is_empty() {
            return;
        }

        let key = (device_name.clone(), os_version.clone());
        let device = self.devices.entry(key).or_insert_with(|| PlotDeviceSamplesBuilder {
            device_name,
            os_version,
            samples_ns: Vec::new(),
            seen_payloads: BTreeSet::new(),
        });
        if !device.seen_payloads.insert(source_signature) {
            return;
        }
        device.samples_ns.extend(samples_ns);
    }

    fn finish(self) -> PlotFunctionInput {
        PlotFunctionInput {
            function_name: self.function_name,
            function_label: self.function_label,
            target: self.target,
            iterations: self.iterations,
            warmup: self.warmup,
            devices: self
                .devices
                .into_values()
                .map(|mut device| {
                    device.samples_ns.sort_unstable();
                    PlotDeviceSamples {
                        device_name: device.device_name,
                        os_version: device.os_version,
                        samples_ns: device.samples_ns,
                    }
                })
                .collect(),
        }
    }
}

fn collect_from_results_dir(
    dir: &Path,
    builders: &mut BTreeMap<(String, String), PlotFunctionInputBuilder>,
) -> Result<()> {
    let mut json_paths = Vec::new();
    collect_json_files(dir, &mut json_paths)?;
    json_paths.sort();

    for path in json_paths {
        let value = read_json(&path)?;
        collect_from_value(&value, &path, builders)?;
    }

    Ok(())
}

fn collect_from_value(
    value: &Value,
    path: &Path,
    builders: &mut BTreeMap<(String, String), PlotFunctionInputBuilder>,
) -> Result<()> {
    if let Some(benchmark_results) = value.get("benchmark_results").and_then(|value| value.as_object()) {
        let (target, iterations, warmup) = extract_run_metadata(value, path);

        for (device_label, entries) in benchmark_results {
            let Some(entries) = entries.as_array() else {
                continue;
            };
            let (device_name, os_version) = parse_device_string(device_label);

            for entry in entries {
                let Some(function_name) = extract_function_name(entry) else {
                    continue;
                };
                let samples_ns = extract_samples_for_plot(entry);
                if samples_ns.is_empty() {
                    continue;
                }
                let source_signature = source_payload_signature(entry);

                let key = (target.clone(), function_name.clone());
                let builder = builders
                    .entry(key)
                    .or_insert_with(|| PlotFunctionInputBuilder::new(function_name, target.clone()));
                builder.set_run_metadata(iterations, warmup);
                builder.add_device_samples(
                    device_name.clone(),
                    os_version.clone(),
                    source_signature,
                    samples_ns,
                );
            }
        }

        return Ok(());
    }

    if value.get("spec").is_some() {
        let (target, iterations, warmup) = extract_run_metadata(value, path);
        let Some(function_name) = extract_function_name(value) else {
            return Ok(());
        };
        let samples_ns = extract_samples_for_plot(value);
        if samples_ns.is_empty() {
            return Ok(());
        }
        let source_signature = source_payload_signature(value);
        let (device_name, os_version) = value
            .get("device")
            .and_then(|value| value.as_str())
            .map(parse_device_string)
            .unwrap_or_else(|| infer_device_from_path(path));

        let key = (target.clone(), function_name.clone());
        let builder = builders
            .entry(key)
            .or_insert_with(|| PlotFunctionInputBuilder::new(function_name, target));
        builder.set_run_metadata(iterations, warmup);
        builder.add_device_samples(device_name, os_version, source_signature, samples_ns);
    }

    Ok(())
}

fn extract_run_metadata(value: &Value, path: &Path) -> (String, u32, u32) {
    let summary = value.get("summary").unwrap_or(value);

    let target = summary
        .get("target")
        .or_else(|| value.get("target"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| infer_target_from_path(path));

    let iterations = summary
        .get("iterations")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;
    let warmup = summary
        .get("warmup")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;

    (target, iterations, warmup)
}

fn extract_function_name(value: &Value) -> Option<String> {
    value
        .get("function")
        .and_then(|value| value.as_str())
        .or_else(|| {
            value
                .get("spec")
                .and_then(|spec| spec.get("name"))
                .and_then(|value| value.as_str())
        })
        .map(str::to_string)
}

fn extract_samples_for_plot(value: &Value) -> Vec<u64> {
    let mut samples = crate::extract_samples(value);
    if samples.is_empty()
        && let Some(samples_ns) = value.get("samples_ns").and_then(|value| value.as_array())
    {
        samples.extend(samples_ns.iter().filter_map(|value| value.as_u64()));
    }
    samples.sort_unstable();
    samples
}

fn parse_device_string(s: &str) -> (String, String) {
    match s.rsplit_once('-') {
        Some((name, version)) if !name.is_empty() && !version.is_empty() => {
            (name.to_string(), version.to_string())
        }
        _ => (s.to_string(), "unknown".to_string()),
    }
}

fn infer_device_from_path(path: &Path) -> (String, String) {
    let lower_path = path.to_string_lossy().to_ascii_lowercase();
    if lower_path.contains("/ios/") || lower_path.contains("\\ios\\") {
        ("unknown".to_string(), "unknown".to_string())
    } else if lower_path.contains("/android/") || lower_path.contains("\\android\\") {
        ("unknown".to_string(), "unknown".to_string())
    } else {
        ("unknown".to_string(), "unknown".to_string())
    }
}

fn infer_target_from_path(path: &Path) -> String {
    let lower_path = path.to_string_lossy().to_ascii_lowercase();
    if lower_path.contains("/ios/") || lower_path.contains("\\ios\\") {
        "ios".to_string()
    } else if lower_path.contains("/android/") || lower_path.contains("\\android\\") {
        "android".to_string()
    } else {
        "unknown".to_string()
    }
}

fn humanize_benchmark_name(name: &str) -> String {
    let leaf = name.rsplit("::").next().unwrap_or(name);
    let s = leaf.strip_prefix("bench_").unwrap_or(leaf);
    s.replace('_', "-")
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("Failed to read results directory {}", dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("Failed to iterate results directory {}", dir.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }

    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))
}

fn write_json(path: &Path, value: Value) {
    fs::write(path, serde_json::to_vec_pretty(&value).expect("serialize json"))
        .expect("write json");
}

fn source_payload_signature(value: &Value) -> String {
    serde_json::to_string(value).expect("serialize source payload")
}
