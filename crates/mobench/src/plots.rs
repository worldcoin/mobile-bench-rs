use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlotFunctionInput {
    pub function_name: String,
    pub function_label: String,
    pub target: String,
    pub iterations: u32,
    pub warmup: u32,
    pub devices: Vec<PlotDeviceSamples>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlotDeviceSamples {
    pub device_name: String,
    pub os_version: String,
    pub samples_ns: Vec<u64>,
}

#[test]
fn extract_function_plot_inputs_reads_fixture_samples() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ci-artifact-root");

    let plots = extract_function_plot_inputs_from_results_dir(&root).expect("extract plot inputs");

    let alpha = plots
        .iter()
        .find(|plot| plot.function_name == "bench_alpha")
        .expect("bench_alpha plot");

    assert_eq!(alpha.devices.len(), 1);
    assert_eq!(alpha.devices[0].device_name, "Google Pixel 8");
    assert_eq!(
        alpha.devices[0].samples_ns,
        vec![
            95_000_000,
            98_000_000,
            100_000_000,
            120_000_000,
            123_000_000
        ]
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

    let plots =
        extract_function_plot_inputs_from_results_dir(root.path()).expect("extract plot inputs");

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
fn extract_function_plot_inputs_preserves_duplicate_runs_from_non_root_files() {
    let root = tempfile::tempdir().expect("tempdir");
    let root_summary = root.path().join("summary.json");
    let nested_dir = root.path().join("android").join("bench_alpha");
    fs::create_dir_all(&nested_dir).expect("create nested dir");

    write_json(
        &root_summary,
        serde_json::json!({
            "summary": {
                "generated_at": "2026-03-25T00:00:00Z",
                "generated_at_unix": 1_000_000_002_u64,
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
                        "mean_ns": 333_u64,
                        "median_ns": 300_u64,
                        "p95_ns": 400_u64,
                        "min_ns": 300_u64,
                        "max_ns": 400_u64
                    }]
                }]
            },
            "benchmark_results": {
                "Google Pixel 8-14.0": [{
                    "function": "bench_beta",
                    "samples": [300_u64, 300_u64, 400_u64]
                }]
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
                "Google Pixel 8-14.0": [
                    {
                        "function": "bench_alpha",
                        "samples": [100_u64, 100_u64, 200_u64]
                    },
                    {
                        "function": "bench_alpha",
                        "samples": [100_u64, 100_u64, 200_u64]
                    }
                ]
            }
        }),
    );

    let plots =
        extract_function_plot_inputs_from_results_dir(root.path()).expect("extract plot inputs");

    let alpha = plots
        .iter()
        .find(|plot| plot.function_name == "bench_alpha")
        .expect("bench_alpha plot");
    let beta = plots
        .iter()
        .find(|plot| plot.function_name == "bench_beta")
        .expect("bench_beta plot");

    assert_eq!(alpha.devices.len(), 1);
    assert_eq!(
        alpha.devices[0].samples_ns,
        vec![100, 100, 100, 100, 200, 200]
    );
    assert_eq!(beta.devices[0].samples_ns, vec![300, 300, 400]);
}

#[test]
fn render_plots_auto_mode_skips_missing_python() {
    let out = tempfile::tempdir().expect("tempdir");
    let inputs = vec![sample_plot_input()];

    let rendered = render_plot_artifacts(
        &inputs,
        out.path(),
        PlotMode::Auto,
        Some(Path::new("/definitely/missing/python")),
    )
    .expect("auto mode should not fail");

    assert!(rendered.is_empty());
}

#[test]
fn materialize_renderer_writes_script_and_style() {
    let out = tempfile::tempdir().expect("tempdir");
    let bundle = materialize_renderer_assets(out.path()).expect("materialize renderer");

    assert!(bundle.script_path.exists());
    assert!(bundle.style_path.exists());
    assert_eq!(
        fs::read_to_string(&bundle.script_path).expect("read script"),
        PLOT_SCRIPT
    );
    assert_eq!(
        fs::read_to_string(&bundle.style_path).expect("read style"),
        PLOT_STYLE
    );
}

#[test]
fn allocate_plot_file_names_deduplicates_function_labels() {
    let first = sample_plot_input();
    let mut second = sample_plot_input();
    second.target = "ios".to_string();

    assert_eq!(
        allocate_plot_file_names(&[first, second]),
        vec![
            "nullifier-proof-generation.svg".to_string(),
            "nullifier-proof-generation-ios.svg".to_string()
        ]
    );
}

#[cfg(unix)]
#[test]
fn render_plot_artifacts_invokes_renderer_with_fake_python() {
    use std::os::unix::fs::PermissionsExt;

    let out = tempfile::tempdir().expect("tempdir");
    let fake_python = out.path().join("fake-python");
    fs::write(
        &fake_python,
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

    let mut permissions = fs::metadata(&fake_python)
        .expect("fake python metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_python, permissions).expect("set fake python perms");

    let rendered = render_plot_artifacts(
        &[sample_plot_input()],
        out.path(),
        PlotMode::Require,
        Some(&fake_python),
    )
    .expect("render plots");

    assert_eq!(rendered.len(), 1);
    assert_eq!(
        rendered[0].relative_path,
        PathBuf::from("plots/nullifier-proof-generation.svg")
    );
    assert_eq!(
        fs::read_to_string(&rendered[0].output_path).expect("read svg"),
        "<svg>ok</svg>"
    );
}

pub fn extract_function_plot_inputs_from_results_dir(dir: &Path) -> Result<Vec<PlotFunctionInput>> {
    let mut builders = BTreeMap::new();
    let mut nested_source_keys = BTreeSet::new();

    collect_from_results_dir(dir, &mut builders, &mut nested_source_keys)?;

    Ok(finish_plot_inputs(builders))
}

pub fn extract_function_plot_inputs_from_output_value(
    value: &Value,
) -> Result<Vec<PlotFunctionInput>> {
    let mut builders = BTreeMap::new();
    let mut nested_source_keys = BTreeSet::new();
    collect_from_output_value(
        value,
        Path::new("summary.json"),
        &mut builders,
        &mut nested_source_keys,
    )?;
    Ok(finish_plot_inputs(builders))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlotMode {
    Auto,
    Off,
    Require,
}

#[derive(Debug)]
pub struct RendererAssets {
    _tempdir: ManagedTempDir,
    pub script_path: PathBuf,
    pub style_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPlot {
    pub function_name: String,
    pub function_label: String,
    pub target: String,
    pub output_path: PathBuf,
    pub relative_path: PathBuf,
}

const PLOT_SCRIPT_NAME: &str = "render_sina_plot.py";
const PLOT_STYLE_NAME: &str = "mobench_light.mplstyle";
const PLOT_SCRIPT: &str = include_str!("../python/render_sina_plot.py");
const PLOT_STYLE: &str = include_str!("../python/mobench_light.mplstyle");

static ASSET_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct ManagedTempDir {
    path: PathBuf,
}

impl ManagedTempDir {
    fn new_in(parent: &Path) -> Result<Self> {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;

        for _ in 0..1024 {
            let seq = ASSET_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(".mobench-plot-assets-{seq}"));
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(Self { path: candidate }),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("creating temporary directory {}", candidate.display())
                    });
                }
            }
        }

        anyhow::bail!("failed to allocate a temporary plot asset directory")
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ManagedTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn materialize_renderer_assets(output_dir: &Path) -> Result<RendererAssets> {
    let tempdir = ManagedTempDir::new_in(output_dir)?;
    let script_path = tempdir.path().join(PLOT_SCRIPT_NAME);
    let style_path = tempdir.path().join(PLOT_STYLE_NAME);

    fs::write(&script_path, PLOT_SCRIPT)
        .with_context(|| format!("writing renderer script {}", script_path.display()))?;
    fs::write(&style_path, PLOT_STYLE)
        .with_context(|| format!("writing renderer style {}", style_path.display()))?;

    Ok(RendererAssets {
        _tempdir: tempdir,
        script_path,
        style_path,
    })
}

pub fn render_plot_artifacts(
    inputs: &[PlotFunctionInput],
    output_dir: &Path,
    mode: PlotMode,
    python_override: Option<&Path>,
) -> Result<Vec<RenderedPlot>> {
    match mode {
        PlotMode::Off => Ok(Vec::new()),
        PlotMode::Auto | PlotMode::Require => {
            if inputs.is_empty() {
                return Ok(Vec::new());
            }

            let assets = materialize_renderer_assets(output_dir)?;
            let plots_dir = output_dir.join("plots");
            fs::create_dir_all(&plots_dir)
                .with_context(|| format!("creating directory {}", plots_dir.display()))?;
            let file_names = allocate_plot_file_names(inputs);

            let mut rendered = Vec::new();
            for (input, file_name) in inputs.iter().zip(file_names.iter()) {
                match render_single_plot(input, &plots_dir, &assets, python_override, file_name) {
                    Ok(plot) => rendered.push(plot),
                    Err(err) if mode == PlotMode::Auto => {
                        eprintln!("Skipping plot {}: {err}", input.function_name);
                    }
                    Err(err) => return Err(err),
                }
            }

            Ok(rendered)
        }
    }
}

fn render_single_plot(
    input: &PlotFunctionInput,
    plots_dir: &Path,
    assets: &RendererAssets,
    python_override: Option<&Path>,
    file_name: &str,
) -> Result<RenderedPlot> {
    let output_path = plots_dir.join(file_name);
    let relative_path = PathBuf::from("plots").join(file_name);
    let input_path = assets
        .script_path
        .parent()
        .context("renderer assets missing parent directory")?
        .join(format!(
            "{}.json",
            slugify_for_filename(&input.function_name)
        ));
    let payload = serde_json::to_vec_pretty(input).context("serializing plot input")?;
    fs::write(&input_path, payload)
        .with_context(|| format!("writing plot input {}", input_path.display()))?;

    let mut last_not_found = None;
    for candidate in python_candidates(python_override) {
        match try_render_with_python(&candidate, &assets.script_path, &input_path, &output_path) {
            Ok(()) => {
                return Ok(RenderedPlot {
                    function_name: input.function_name.clone(),
                    function_label: input.function_label.clone(),
                    target: input.target.clone(),
                    output_path,
                    relative_path,
                });
            }
            Err(RenderAttemptError::NotFound(err)) => last_not_found = Some(err),
            Err(RenderAttemptError::Failed(err)) => return Err(err),
        }
    }

    Err(last_not_found
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("no usable python interpreter found")))
}

enum RenderAttemptError {
    NotFound(io::Error),
    Failed(anyhow::Error),
}

fn try_render_with_python(
    python: &Path,
    script_path: &Path,
    input_path: &Path,
    output_path: &Path,
) -> std::result::Result<(), RenderAttemptError> {
    let output = Command::new(python)
        .arg(script_path)
        .arg("--input")
        .arg(input_path)
        .arg("--output")
        .arg(output_path)
        .output();

    let output = match output {
        Ok(output) => output,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(RenderAttemptError::NotFound(err));
        }
        Err(err) => return Err(RenderAttemptError::Failed(err.into())),
    };

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let details = match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => "renderer exited unsuccessfully".to_string(),
        (false, true) => format!("renderer stdout: {stdout}"),
        (true, false) => format!("renderer stderr: {stderr}"),
        (false, false) => format!("renderer stdout: {stdout}; stderr: {stderr}"),
    };

    Err(RenderAttemptError::Failed(anyhow::anyhow!(
        "{} failed for {}",
        python.display(),
        details
    )))
}

fn python_candidates(python_override: Option<&Path>) -> Vec<PathBuf> {
    if let Some(path) = python_override {
        return vec![path.to_path_buf()];
    }

    let mut candidates = Vec::new();
    if let Ok(value) = std::env::var("MOBENCH_PLOT_PYTHON") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }
    candidates.push(PathBuf::from("python3"));
    candidates.push(PathBuf::from("python"));
    candidates
}

fn allocate_plot_file_names(inputs: &[PlotFunctionInput]) -> Vec<String> {
    let mut used = BTreeSet::new();
    inputs
        .iter()
        .map(|input| {
            let base = slugify_for_filename(&input.function_label);
            let target = slugify_for_filename(&input.target);
            let function = slugify_for_filename(&input.function_name);

            for candidate in [
                Some(base.clone()),
                (!target.is_empty()).then(|| format!("{base}-{target}")),
                (function != base && !function.is_empty()).then(|| format!("{base}-{function}")),
            ]
            .into_iter()
            .flatten()
            {
                if used.insert(candidate.clone()) {
                    return format!("{candidate}.svg");
                }
            }

            let mut index = 2usize;
            loop {
                let candidate = format!("{base}-{index}");
                if used.insert(candidate.clone()) {
                    return format!("{candidate}.svg");
                }
                index += 1;
            }
        })
        .collect()
}

fn slugify_for_filename(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;

    for ch in value.to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }

    while slug.starts_with('-') {
        slug.remove(0);
    }
    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "plot".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
fn sample_plot_input() -> PlotFunctionInput {
    PlotFunctionInput {
        function_name: "bench_nullifier_proof_generation".to_string(),
        function_label: "nullifier-proof-generation".to_string(),
        target: "android".to_string(),
        iterations: 20,
        warmup: 5,
        devices: vec![PlotDeviceSamples {
            device_name: "iPhone 15".to_string(),
            os_version: "iOS 17.4".to_string(),
            samples_ns: vec![10_000_000, 11_000_000],
        }],
    }
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
        samples_ns: Vec<u64>,
    ) {
        if samples_ns.is_empty() {
            return;
        }

        let key = (device_name.clone(), os_version.clone());
        let device = self
            .devices
            .entry(key)
            .or_insert_with(|| PlotDeviceSamplesBuilder {
                device_name,
                os_version,
                samples_ns: Vec::new(),
            });
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
    nested_source_keys: &mut BTreeSet<PlotEntryKey>,
) -> Result<()> {
    let mut json_paths = Vec::new();
    collect_json_files(dir, &mut json_paths)?;
    json_paths.sort();

    let root_summary_path = dir.join("summary.json");
    let mut nested_paths = Vec::new();
    let mut root_paths = Vec::new();
    for path in json_paths {
        if path == root_summary_path {
            root_paths.push(path);
        } else {
            nested_paths.push(path);
        }
    }

    for path in nested_paths {
        let value = read_json(&path)?;
        collect_from_value(&value, &path, false, builders, nested_source_keys)?;
    }

    for path in root_paths {
        let value = read_json(&path)?;
        collect_from_value(&value, &path, true, builders, nested_source_keys)?;
    }

    Ok(())
}

fn collect_from_output_value(
    value: &Value,
    path: &Path,
    builders: &mut BTreeMap<(String, String), PlotFunctionInputBuilder>,
    nested_source_keys: &mut BTreeSet<PlotEntryKey>,
) -> Result<()> {
    collect_from_value(value, path, false, builders, nested_source_keys)?;

    if let Some(functions) = value
        .get("functions")
        .and_then(|functions| functions.as_object())
    {
        for entry in functions.values() {
            collect_from_output_value(entry, path, builders, nested_source_keys)?;
        }
    }

    if let Some(targets) = value.get("targets").and_then(|targets| targets.as_object()) {
        for entry in targets.values() {
            collect_from_output_value(entry, path, builders, nested_source_keys)?;
        }
    }

    Ok(())
}

fn finish_plot_inputs(
    builders: BTreeMap<(String, String), PlotFunctionInputBuilder>,
) -> Vec<PlotFunctionInput> {
    let mut plots = builders
        .into_values()
        .map(PlotFunctionInputBuilder::finish)
        .collect::<Vec<_>>();
    plots.sort_by(|left, right| {
        left.target
            .cmp(&right.target)
            .then(left.function_name.cmp(&right.function_name))
    });
    plots
}

fn collect_from_value(
    value: &Value,
    path: &Path,
    is_root_summary: bool,
    builders: &mut BTreeMap<(String, String), PlotFunctionInputBuilder>,
    nested_source_keys: &mut BTreeSet<PlotEntryKey>,
) -> Result<()> {
    if let Some(benchmark_results) = value
        .get("benchmark_results")
        .and_then(|value| value.as_object())
    {
        let (target, iterations, warmup) = extract_run_metadata(value, path);

        for (device_label, entries) in benchmark_results {
            let Some(entries) = entries.as_array() else {
                continue;
            };

            for entry in entries {
                let Some(function_name) = extract_function_name(entry) else {
                    continue;
                };
                let samples_ns = extract_samples_for_plot(entry);
                if samples_ns.is_empty() {
                    continue;
                }
                let (device_name, os_version) = parse_device_string(device_label);
                let entry_key = PlotEntryKey::new(
                    &target,
                    &function_name,
                    &device_name,
                    &os_version,
                    iterations,
                    warmup,
                    &samples_ns,
                );
                if is_root_summary && nested_source_keys.contains(&entry_key) {
                    continue;
                }
                if !is_root_summary {
                    nested_source_keys.insert(entry_key);
                }

                let key = (target.clone(), function_name.clone());
                let builder = builders.entry(key).or_insert_with(|| {
                    PlotFunctionInputBuilder::new(function_name, target.clone())
                });
                builder.set_run_metadata(iterations, warmup);
                builder.add_device_samples(device_name, os_version, samples_ns);
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
        let (device_name, os_version) = value
            .get("device")
            .and_then(|value| value.as_str())
            .map(parse_device_string)
            .unwrap_or_else(|| infer_device_from_path(path));
        let entry_key = PlotEntryKey::new(
            &target,
            &function_name,
            &device_name,
            &os_version,
            iterations,
            warmup,
            &samples_ns,
        );
        if is_root_summary && nested_source_keys.contains(&entry_key) {
            return Ok(());
        }
        if !is_root_summary {
            nested_source_keys.insert(entry_key);
        }

        let key = (target.clone(), function_name.clone());
        let builder = builders
            .entry(key)
            .or_insert_with(|| PlotFunctionInputBuilder::new(function_name, target));
        builder.set_run_metadata(iterations, warmup);
        builder.add_device_samples(device_name, os_version, samples_ns);
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
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

#[cfg(test)]
fn write_json(path: &Path, value: Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&value).expect("serialize json"),
    )
    .expect("write json");
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PlotEntryKey {
    target: String,
    function_name: String,
    device_name: String,
    os_version: String,
    iterations: u32,
    warmup: u32,
    samples_ns: Vec<u64>,
}

impl PlotEntryKey {
    fn new(
        target: &str,
        function_name: &str,
        device_name: &str,
        os_version: &str,
        iterations: u32,
        warmup: u32,
        samples_ns: &[u64],
    ) -> Self {
        Self {
            target: target.to_string(),
            function_name: function_name.to_string(),
            device_name: device_name.to_string(),
            os_version: os_version.to_string(),
            iterations,
            warmup,
            samples_ns: samples_ns.to_vec(),
        }
    }
}
