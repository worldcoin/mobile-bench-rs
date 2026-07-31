//! Project discovery, layout resolution, and typed builder configuration.
//!
//! This Module concentrates the policy that maps a caller's paths and
//! mobench.toml into one ResolvedProjectLayout. Build and command Modules use
//! that resolved value instead of repeating discovery or configuration rules.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::config;
use crate::process_adapter::ToolCommand;
use crate::{
    BenchConfig, BrowserStackConfig, DeviceEntry, DeviceMatrix, IosRunnerArg, IosXcuitestArtifacts,
    MobileTarget, ResolvedProjectLayout, ensure_can_write, write_file,
};

// Builder settings added after the released public config structs became
// constructible API. Keep them in a private deserialization layer so adding
// TOML keys does not add fields to those exhaustive public structs.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LayoutConfigExtensions {
    project: LayoutProjectExtensions,
    ios: LayoutIosExtensions,
    browserstack: LayoutBrowserStackExtensions,
    web: LayoutWebExtensions,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LayoutProjectExtensions {
    ffi_backend: Option<mobench_sdk::FfiBackend>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LayoutIosExtensions {
    runner: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LayoutBrowserStackExtensions {
    android_benchmark_timeout_secs: Option<u64>,
    android_heartbeat_interval_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LayoutWebExtensions {
    wasm_bindgen: Option<PathBuf>,
}

fn load_layout_config_extensions(path: &Path) -> Result<LayoutConfigExtensions> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProjectLayoutOptions<'a> {
    pub(crate) start_dir: Option<&'a Path>,
    pub(crate) project_root: Option<&'a Path>,
    pub(crate) crate_path: Option<&'a Path>,
    pub(crate) config_path: Option<&'a Path>,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataPackage {
    name: String,
    manifest_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataOutput {
    workspace_root: PathBuf,
    packages: Vec<CargoMetadataPackage>,
}

fn canonicalize_from(base: &Path, path: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    joined
        .canonicalize()
        .with_context(|| format!("resolving path {}", joined.display()))
}

fn resolve_existing_path_arg(base: &Path, path: Option<&Path>) -> Result<Option<PathBuf>> {
    path.map(|value| canonicalize_from(base, value)).transpose()
}

fn cargo_metadata_from(start: &Path) -> Option<CargoMetadataOutput> {
    let mut command = ToolCommand::path_search("cargo");
    command
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(start)
        .timeout(Duration::from_secs(30));
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn git_root_from(start: &Path) -> Option<PathBuf> {
    let mut command = ToolCommand::path_search("git");
    command
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .timeout(Duration::from_secs(30));
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let path = stdout.trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn config_discovery_base(
    start_dir: &Path,
    explicit_project_root: Option<&PathBuf>,
    explicit_crate_path: Option<&PathBuf>,
) -> PathBuf {
    explicit_project_root
        .cloned()
        .or_else(|| explicit_crate_path.cloned())
        .unwrap_or_else(|| start_dir.to_path_buf())
}

fn load_layout_config(
    start_dir: &Path,
    explicit_project_root: Option<&PathBuf>,
    explicit_crate_path: Option<&PathBuf>,
    explicit_config_path: Option<&PathBuf>,
) -> Result<Option<(config::MobenchConfig, PathBuf)>> {
    if let Some(path) = explicit_config_path {
        return Ok(Some((
            config::MobenchConfig::load_from_file(path)?,
            path.to_path_buf(),
        )));
    }

    let discovery_base =
        config_discovery_base(start_dir, explicit_project_root, explicit_crate_path);
    config::MobenchConfig::discover_from(&discovery_base)
}

fn resolve_project_root_for_layout(
    start_dir: &Path,
    explicit_project_root: Option<PathBuf>,
    explicit_crate_path: Option<&PathBuf>,
    config_path: Option<&Path>,
) -> PathBuf {
    if let Some(root) = explicit_project_root {
        return root;
    }
    if let Some(path) = config_path
        && let Some(parent) = path.parent()
    {
        return parent.to_path_buf();
    }
    if let Some(crate_path) = explicit_crate_path
        && let Some(metadata) = cargo_metadata_from(crate_path)
    {
        return metadata.workspace_root;
    }
    if let Some(metadata) = cargo_metadata_from(start_dir) {
        return metadata.workspace_root;
    }
    if let Some(crate_path) = explicit_crate_path
        && let Some(root) = git_root_from(crate_path)
    {
        return root;
    }
    if let Some(root) = git_root_from(start_dir) {
        return root;
    }
    start_dir.to_path_buf()
}

fn read_package_name_from_dir(dir: &Path) -> Option<String> {
    mobench_sdk::builders::common::read_package_name(&dir.join("Cargo.toml"))
}

fn package_dir_from_metadata(metadata: &CargoMetadataOutput, crate_name: &str) -> Option<PathBuf> {
    metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == crate_name)
        .and_then(|pkg| pkg.manifest_path.parent().map(Path::to_path_buf))
}

fn resolve_configured_crate_dir(project_root: &Path, crate_name: &str) -> Result<Option<PathBuf>> {
    if let Some(pkg_name) = read_package_name_from_dir(project_root)
        && pkg_name == crate_name
    {
        return Ok(Some(project_root.to_path_buf()));
    }

    if let Some(metadata) = cargo_metadata_from(project_root)
        && let Some(dir) = package_dir_from_metadata(&metadata, crate_name)
    {
        return Ok(Some(dir));
    }

    let candidates = [
        project_root.join("crates").join(crate_name),
        project_root.join(crate_name),
        project_root.join("bench-mobile"),
    ];

    for candidate in candidates {
        let manifest = candidate.join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        if read_package_name_from_dir(&candidate).as_deref() == Some(crate_name) {
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

fn resolve_legacy_crate_dir(project_root: &Path) -> Result<PathBuf> {
    let candidates = [
        project_root.to_path_buf(),
        project_root.join("bench-mobile"),
        project_root.join("crates/sample-fns"),
    ];

    for candidate in candidates {
        let manifest = candidate.join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        if read_package_name_from_dir(&candidate).is_some() {
            return Ok(candidate);
        }
    }

    bail!(
        "No benchmark crate found. Pass --crate-path, set [project].crate in mobench.toml, or use a legacy bench-mobile layout."
    )
}

pub(crate) fn resolve_project_layout(
    options: ProjectLayoutOptions<'_>,
) -> Result<ResolvedProjectLayout> {
    let start_dir = match options.start_dir {
        Some(path) => canonicalize_from(Path::new("."), path)?,
        None => std::env::current_dir().context("Failed to get current directory")?,
    };
    let explicit_project_root = resolve_existing_path_arg(&start_dir, options.project_root)?;
    let explicit_crate_path = resolve_existing_path_arg(&start_dir, options.crate_path)?;
    let explicit_config_path = resolve_existing_path_arg(&start_dir, options.config_path)?;

    let loaded_config = load_layout_config(
        &start_dir,
        explicit_project_root.as_ref(),
        explicit_crate_path.as_ref(),
        explicit_config_path.as_ref(),
    )?;
    let (config, config_path) = match loaded_config {
        Some((config, path)) => (Some(config), Some(path)),
        None => (None, None),
    };
    let extensions = config_path
        .as_deref()
        .map(load_layout_config_extensions)
        .transpose()?
        .unwrap_or_default();

    let project_root = resolve_project_root_for_layout(
        &start_dir,
        explicit_project_root,
        explicit_crate_path.as_ref(),
        config_path.as_deref(),
    );

    let crate_dir = if let Some(crate_path) = explicit_crate_path {
        crate_path
    } else if let Some(configured_name) = config
        .as_ref()
        .and_then(|cfg| cfg.project.crate_name.as_deref())
    {
        resolve_configured_crate_dir(&project_root, configured_name)?.ok_or_else(|| {
            anyhow!(
                "Configured benchmark crate '{}' was not found under {}",
                configured_name,
                project_root.display()
            )
        })?
    } else {
        resolve_legacy_crate_dir(&project_root)?
    };

    let crate_name = read_package_name_from_dir(&crate_dir).ok_or_else(|| {
        anyhow!(
            "package.name not found in {}",
            crate_dir.join("Cargo.toml").display()
        )
    })?;
    let library_name = config
        .as_ref()
        .and_then(|cfg| cfg.library_name())
        .unwrap_or_else(|| crate_name.replace('-', "_"));
    let ffi_backend = extensions.project.ffi_backend.unwrap_or_default();
    let android_abis = config.as_ref().and_then(|cfg| cfg.android.abis.clone());
    let ios_completion_timeout_secs = config
        .as_ref()
        .and_then(|cfg| cfg.browserstack.ios_completion_timeout_secs);
    let ios_deployment_target = config
        .as_ref()
        .map(|cfg| cfg.ios.deployment_target.clone())
        .unwrap_or_else(|| mobench_sdk::codegen::DEFAULT_IOS_DEPLOYMENT_TARGET.to_string());
    let ios_runner = extensions.ios.runner;
    let android_benchmark_timeout_secs = extensions.browserstack.android_benchmark_timeout_secs;
    let android_heartbeat_interval_secs = extensions.browserstack.android_heartbeat_interval_secs;
    let web_wasm_bindgen = extensions.web.wasm_bindgen;
    let configured_output_dir = config
        .as_ref()
        .and_then(|cfg| cfg.project.output_dir.as_deref())
        .unwrap_or_else(|| Path::new("target/mobench"));
    let approved_project_root =
        mobench_artifacts::ApprovedRoot::existing(&project_root).map_err(|error| {
            anyhow!("config_error: project root cannot own generated artifacts: {error}")
        })?;
    let output_dir = approved_project_root
        .project_dir(configured_output_dir)
        .map_err(|error| {
            anyhow!(
                "config_error: project.output_dir must be a relative, non-symlinked directory beneath {}: {error}",
                project_root.display()
            )
        })?;
    let default_function = config
        .as_ref()
        .and_then(|cfg| cfg.benchmarks.default_function.clone());

    Ok(ResolvedProjectLayout {
        project_root,
        crate_dir,
        crate_name,
        library_name,
        ffi_backend,
        android_abis,
        ios_completion_timeout_secs,
        ios_deployment_target,
        ios_runner,
        android_benchmark_timeout_secs,
        android_heartbeat_interval_secs,
        web_wasm_bindgen,
        config_path,
        output_dir,
        default_function,
    })
}

pub(crate) fn discover_benchmarks_for_layout(
    layout: &ResolvedProjectLayout,
) -> Result<Vec<String>> {
    let mut benchmarks =
        mobench_sdk::codegen::detect_all_benchmarks(&layout.crate_dir, &layout.crate_name);
    benchmarks.sort();
    benchmarks.dedup();
    Ok(benchmarks)
}

pub(crate) fn ensure_verify_smoke_test_supported(layout: &ResolvedProjectLayout) -> Result<()> {
    let supported_embedded_crates = ["sample-fns", "basic-benchmark", "ffi-benchmark"];
    if supported_embedded_crates.contains(&layout.crate_name.as_str()) {
        return Ok(());
    }

    bail!(
        "verify --smoke-test is unsupported for external crate '{}'; smoke tests only work for benchmark crates linked into the mobench CLI binary",
        layout.crate_name
    )
}

pub(crate) fn configured_android_abis(layout: &ResolvedProjectLayout) -> Vec<String> {
    layout
        .android_abis
        .as_ref()
        .filter(|abis| !abis.is_empty())
        .cloned()
        .unwrap_or_else(|| vec!["arm64-v8a".to_string()])
}

pub(crate) fn configured_ios_completion_timeout_secs(
    layout: &ResolvedProjectLayout,
    ios_completion_timeout_secs: Option<u64>,
) -> Option<u64> {
    ios_completion_timeout_secs.or(layout.ios_completion_timeout_secs)
}

pub(crate) fn configured_ios_deployment_target(
    layout: &ResolvedProjectLayout,
    ios_deployment_target: Option<&str>,
) -> Result<mobench_sdk::codegen::IosDeploymentTarget> {
    let raw = ios_deployment_target.unwrap_or(&layout.ios_deployment_target);
    mobench_sdk::codegen::IosDeploymentTarget::parse(raw)
        .map_err(|err| anyhow!("config_error: {err}"))
}

pub(crate) fn configured_ios_runner(
    layout: &ResolvedProjectLayout,
    deployment_target: &mobench_sdk::codegen::IosDeploymentTarget,
    ios_runner: Option<&str>,
) -> Result<mobench_sdk::codegen::IosRunner> {
    let requested = if let Some(raw_runner) = ios_runner {
        Some(
            mobench_sdk::codegen::IosRunner::parse(raw_runner)
                .map_err(|err| anyhow!("config_error: {err}"))?,
        )
    } else {
        layout
            .ios_runner
            .as_deref()
            .map(mobench_sdk::codegen::IosRunner::parse)
            .transpose()
            .map_err(|err| anyhow!("config_error: {err}"))?
    };
    mobench_sdk::codegen::resolve_ios_runner(deployment_target, requested)
        .map_err(|err| anyhow!("config_error: {err}"))
}

pub(crate) fn ios_runner_arg_name(runner: IosRunnerArg) -> &'static str {
    match runner {
        IosRunnerArg::Swiftui => "swiftui",
        IosRunnerArg::UikitLegacy => "uikit-legacy",
    }
}

pub(crate) fn configured_android_benchmark_timeout_secs(
    layout: &ResolvedProjectLayout,
    android_benchmark_timeout_secs: Option<u64>,
) -> Option<u64> {
    android_benchmark_timeout_secs.or(layout.android_benchmark_timeout_secs)
}

pub(crate) fn configured_android_heartbeat_interval_secs(
    layout: &ResolvedProjectLayout,
    android_heartbeat_interval_secs: Option<u64>,
) -> Option<u64> {
    android_heartbeat_interval_secs.or(layout.android_heartbeat_interval_secs)
}

pub(crate) fn android_builder_for_layout(
    layout: &ResolvedProjectLayout,
) -> mobench_sdk::builders::AndroidBuilder {
    mobench_sdk::builders::AndroidBuilder::new(&layout.project_root, layout.crate_name.clone())
        .ffi_backend(layout.ffi_backend)
}

pub(crate) fn ios_builder_for_layout(
    layout: &ResolvedProjectLayout,
) -> mobench_sdk::builders::IosBuilder {
    mobench_sdk::builders::IosBuilder::new(&layout.project_root, layout.crate_name.clone())
        .ffi_backend(layout.ffi_backend)
}

pub(crate) fn write_config_template(
    path: &Path,
    target: MobileTarget,
    overwrite: bool,
) -> Result<()> {
    ensure_can_write(path, overwrite)?;

    let ios_xcuitest = if target == MobileTarget::Ios {
        Some(IosXcuitestArtifacts {
            app: PathBuf::from("target/ios/BenchRunner.ipa"),
            test_suite: PathBuf::from("target/ios/BenchRunnerUITests.zip"),
        })
    } else {
        None
    };

    let cfg = BenchConfig {
        target,
        function: "sample_fns::fibonacci".into(),
        iterations: 100,
        warmup: 10,
        device_matrix: PathBuf::from("device-matrix.yaml"),
        device_tags: Some(vec!["default".into()]),
        browserstack: BrowserStackConfig {
            app_automate_username: "${BROWSERSTACK_USERNAME}".into(),
            app_automate_access_key: "${BROWSERSTACK_ACCESS_KEY}".into(),
            project: Some("mobile-bench-rs".into()),
            ios_completion_timeout_secs: None,
            android_benchmark_timeout_secs: None,
            android_heartbeat_interval_secs: None,
        },
        ios_xcuitest,
    };

    let contents = toml::to_string_pretty(&cfg)?;
    write_file(path, contents.as_bytes())
}

pub(crate) fn write_device_matrix_template(path: &Path, overwrite: bool) -> Result<()> {
    ensure_can_write(path, overwrite)?;

    let matrix = DeviceMatrix {
        devices: vec![
            DeviceEntry {
                name: "Pixel 7".into(),
                os: "android".into(),
                os_version: "13.0".into(),
                tags: Some(vec!["default".into(), "pixel".into()]),
            },
            DeviceEntry {
                name: "iPhone 14".into(),
                os: "ios".into(),
                os_version: "16".into(),
                tags: Some(vec!["default".into(), "iphone".into()]),
            },
        ],
    };

    let contents = serde_yaml::to_string(&matrix)?;
    write_file(path, contents.as_bytes())
}
