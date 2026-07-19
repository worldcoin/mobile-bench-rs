//! Code generation and template management
//!
//! This module provides functionality for generating mobile app projects from
//! embedded templates. It handles template parameterization and file generation.

use crate::types::{BenchError, InitConfig, Target};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use include_dir::{Dir, DirEntry, include_dir};

const ANDROID_TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates/android");
const IOS_TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates/ios");
const NATIVE_ANDROID_MAIN_ACTIVITY_TEMPLATE: &str =
    include_str!("native_templates/android/MainActivity.kt.template");
const NATIVE_IOS_BENCH_RUNNER_FFI_TEMPLATE: &str =
    include_str!("native_templates/ios/BenchRunnerFFI.swift.template");
const BOLTFFI_ANDROID_MAIN_ACTIVITY_TEMPLATE: &str =
    include_str!("boltffi_templates/android/MainActivity.kt.template");
const BOLTFFI_IOS_BENCH_RUNNER_FFI_TEMPLATE: &str =
    include_str!("boltffi_templates/ios/BenchRunnerFFI.swift.template");
pub const DEFAULT_IOS_BENCHMARK_TIMEOUT_SECS: u64 = 300;
const DEFAULT_ANDROID_BENCHMARK_TIMEOUT_SECS: u64 = 1800;
const DEFAULT_ANDROID_HEARTBEAT_INTERVAL_SECS: u64 = 10;

pub const DEFAULT_IOS_DEPLOYMENT_TARGET: &str = "15.0";
pub const SWIFTUI_RUNNER_MIN_IOS: &str = "15.0";

/// Supported generated iOS application runner templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IosRunner {
    /// Current SwiftUI runner. This is the default for iOS 15+.
    Swiftui,
    /// UIKit-based runner for legacy deployment targets.
    UikitLegacy,
}

impl IosRunner {
    pub fn parse(value: &str) -> Result<Self, BenchError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "swiftui" => Ok(Self::Swiftui),
            "uikit-legacy" | "uikit_legacy" => Ok(Self::UikitLegacy),
            other => Err(BenchError::Build(format!(
                "Unsupported iOS runner `{other}`. Supported values: swiftui, uikit-legacy"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Swiftui => "swiftui",
            Self::UikitLegacy => "uikit-legacy",
        }
    }
}

/// Parsed iOS deployment target used for explicit compatibility decisions.
#[derive(Debug, Clone, Eq)]
pub struct IosDeploymentTarget {
    raw: String,
    major: u16,
    minor: u16,
}

impl PartialEq for IosDeploymentTarget {
    fn eq(&self, other: &Self) -> bool {
        (self.major, self.minor) == (other.major, other.minor)
    }
}

impl Ord for IosDeploymentTarget {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor).cmp(&(other.major, other.minor))
    }
}

impl PartialOrd for IosDeploymentTarget {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl IosDeploymentTarget {
    pub fn parse(raw: &str) -> Result<Self, BenchError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(BenchError::Build(
                "iOS deployment target must not be empty".to_string(),
            ));
        }

        let mut parts = raw.split('.');
        let major_raw = parts.next().unwrap_or_default();
        let minor_raw = parts.next().unwrap_or("0");
        if parts.next().is_some() {
            return Err(BenchError::Build(format!(
                "Invalid iOS deployment target `{raw}`. Expected VERSION like 15.0"
            )));
        }

        Ok(Self {
            raw: raw.to_string(),
            major: parse_ios_version_part(raw, major_raw, "major")?,
            minor: parse_ios_version_part(raw, minor_raw, "minor")?,
        })
    }

    pub fn default_target() -> Self {
        Self::parse(DEFAULT_IOS_DEPLOYMENT_TARGET)
            .expect("default iOS deployment target should be valid")
    }
}

fn parse_ios_version_part(raw: &str, part: &str, label: &str) -> Result<u16, BenchError> {
    if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(BenchError::Build(format!(
            "Invalid iOS deployment target `{raw}`: {label} version component must be numeric"
        )));
    }

    part.parse::<u16>().map_err(|err| {
        BenchError::Build(format!(
            "Invalid iOS deployment target `{raw}`: failed to parse {label} component: {err}"
        ))
    })
}

impl std::fmt::Display for IosDeploymentTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw)
    }
}

/// Fully resolved iOS project generation options.
#[derive(Debug, Clone)]
pub struct IosProjectOptions {
    pub deployment_target: IosDeploymentTarget,
    pub runner: IosRunner,
    pub ios_benchmark_timeout_secs: u64,
}

impl Default for IosProjectOptions {
    fn default() -> Self {
        Self {
            deployment_target: IosDeploymentTarget::default_target(),
            runner: IosRunner::Swiftui,
            ios_benchmark_timeout_secs: DEFAULT_IOS_BENCHMARK_TIMEOUT_SECS,
        }
    }
}

pub fn resolve_ios_runner(
    deployment_target: &IosDeploymentTarget,
    requested_runner: Option<IosRunner>,
) -> Result<IosRunner, BenchError> {
    let swiftui_floor = IosDeploymentTarget::parse(SWIFTUI_RUNNER_MIN_IOS)?;
    match requested_runner {
        Some(IosRunner::Swiftui) if deployment_target < &swiftui_floor => {
            Err(BenchError::Build(format!(
                "iOS runner `swiftui` requires deployment target {SWIFTUI_RUNNER_MIN_IOS}+; \
                 requested deployment target is {deployment_target}. Use `uikit-legacy` for older iOS targets."
            )))
        }
        Some(runner) => Ok(runner),
        None if deployment_target < &swiftui_floor => Ok(IosRunner::UikitLegacy),
        None => Ok(IosRunner::Swiftui),
    }
}

/// Template variable that can be replaced in template files
#[derive(Debug, Clone)]
pub struct TemplateVar {
    pub name: &'static str,
    pub value: String,
}

/// Generates a new mobile benchmark project from templates
///
/// Creates the necessary directory structure and files for benchmarking on
/// mobile platforms. This includes:
/// - A `bench-mobile/` crate for FFI bindings
/// - Platform-specific app projects (Android and/or iOS)
/// - Configuration files
///
/// # Arguments
///
/// * `config` - Configuration for project initialization
///
/// # Returns
///
/// * `Ok(PathBuf)` - Path to the generated project root
/// * `Err(BenchError)` - If generation fails
pub fn generate_project(config: &InitConfig) -> Result<PathBuf, BenchError> {
    let output_dir = &config.output_dir;
    let project_slug = sanitize_package_name(&config.project_name);
    let project_pascal = to_pascal_case(&project_slug);
    // Use sanitized bundle ID component (alphanumeric only) to avoid iOS validation issues
    let bundle_id_component = sanitize_bundle_id_component(&project_slug);
    let bundle_prefix = format!("dev.world.{}", bundle_id_component);

    // Create base directories
    fs::create_dir_all(output_dir)?;

    // Generate bench-mobile FFI wrapper crate
    generate_bench_mobile_crate(output_dir, &project_slug)?;

    // For full project generation (init), use "example_fibonacci" as the default
    // since the generated example benchmarks include this function
    let default_function = "example_fibonacci";

    // Generate platform-specific projects
    match config.target {
        Target::Android => {
            generate_android_project(output_dir, &project_slug, default_function)?;
        }
        Target::Ios => {
            generate_ios_project(
                output_dir,
                &project_slug,
                &project_pascal,
                &bundle_prefix,
                default_function,
            )?;
        }
        Target::Both => {
            generate_android_project(output_dir, &project_slug, default_function)?;
            generate_ios_project(
                output_dir,
                &project_slug,
                &project_pascal,
                &bundle_prefix,
                default_function,
            )?;
        }
    }

    // Generate config file
    generate_config_file(output_dir, config)?;

    // Generate examples if requested
    if config.generate_examples {
        generate_example_benchmarks(output_dir)?;
    }

    Ok(output_dir.clone())
}

/// Generates the bench-mobile FFI wrapper crate
fn generate_bench_mobile_crate(output_dir: &Path, project_name: &str) -> Result<(), BenchError> {
    let crate_dir = output_dir.join("bench-mobile");
    fs::create_dir_all(crate_dir.join("src"))?;

    let crate_name = format!("{}-bench-mobile", project_name);

    // Generate Cargo.toml
    // Note: We configure rustls to use 'ring' instead of 'aws-lc-rs' (default in rustls 0.23+)
    // because aws-lc-rs doesn't compile for Android NDK targets.
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[dependencies]
mobench-sdk = {{ path = "..", default-features = false, features = ["registry"] }}
uniffi = "0.28"
{} = {{ path = ".." }}

[features]
default = []

[build-dependencies]
uniffi = {{ version = "0.28", features = ["build"] }}

# Binary for generating UniFFI bindings (used by mobench build)
[[bin]]
name = "uniffi-bindgen"
path = "src/bin/uniffi-bindgen.rs"

# IMPORTANT: If your project uses rustls (directly or transitively), you must configure
# it to use the 'ring' crypto backend instead of 'aws-lc-rs' (the default in rustls 0.23+).
# aws-lc-rs doesn't compile for Android NDK targets due to C compilation issues.
#
# Add this to your root Cargo.toml:
# [workspace.dependencies]
# rustls = {{ version = "0.23", default-features = false, features = ["ring", "std", "tls12"] }}
#
# Then in each crate that uses rustls:
# [dependencies]
# rustls = {{ workspace = true }}
"#,
        crate_name, project_name
    );

    fs::write(crate_dir.join("Cargo.toml"), cargo_toml)?;

    // Generate src/lib.rs
    let lib_rs_template = r#"//! Mobile FFI bindings for benchmarks
//!
//! This crate provides the FFI boundary between Rust benchmarks and mobile
//! platforms (Android/iOS). It uses UniFFI to generate type-safe bindings.

use uniffi;

// Ensure the user crate is linked so benchmark registrations are pulled in.
extern crate {{USER_CRATE}} as _bench_user_crate;

// Re-export mobench-sdk types with UniFFI annotations
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct BenchSpec {
    pub name: String,
    pub iterations: u32,
    pub warmup: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct BenchSample {
    pub duration_ns: u64,
    pub cpu_time_ms: Option<u64>,
    pub peak_memory_kb: Option<u64>,
    pub process_peak_memory_kb: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct SemanticPhase {
    pub name: String,
    pub duration_ns: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct HarnessTimelineSpan {
    pub phase: String,
    pub start_offset_ns: u64,
    pub end_offset_ns: u64,
    pub iteration: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct BenchReport {
    pub spec: BenchSpec,
    pub samples: Vec<BenchSample>,
    pub phases: Vec<SemanticPhase>,
    pub timeline: Vec<HarnessTimelineSpan>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum BenchError {
    #[error("iterations must be greater than zero")]
    InvalidIterations,

    #[error("unknown benchmark function: {name}")]
    UnknownFunction { name: String },

    #[error("benchmark execution failed: {reason}")]
    ExecutionFailed { reason: String },
}

// Convert from mobench-sdk types
impl From<mobench_sdk::BenchSpec> for BenchSpec {
    fn from(spec: mobench_sdk::BenchSpec) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

impl From<BenchSpec> for mobench_sdk::BenchSpec {
    fn from(spec: BenchSpec) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

impl From<mobench_sdk::BenchSample> for BenchSample {
    fn from(sample: mobench_sdk::BenchSample) -> Self {
        Self {
            duration_ns: sample.duration_ns,
            cpu_time_ms: sample.cpu_time_ms,
            peak_memory_kb: sample.peak_memory_kb,
            process_peak_memory_kb: sample.process_peak_memory_kb,
        }
    }
}

impl From<mobench_sdk::SemanticPhase> for SemanticPhase {
    fn from(phase: mobench_sdk::SemanticPhase) -> Self {
        Self {
            name: phase.name,
            duration_ns: phase.duration_ns,
        }
    }
}

impl From<mobench_sdk::HarnessTimelineSpan> for HarnessTimelineSpan {
    fn from(span: mobench_sdk::HarnessTimelineSpan) -> Self {
        Self {
            phase: span.phase,
            start_offset_ns: span.start_offset_ns,
            end_offset_ns: span.end_offset_ns,
            iteration: span.iteration,
        }
    }
}

impl From<mobench_sdk::RunnerReport> for BenchReport {
    fn from(report: mobench_sdk::RunnerReport) -> Self {
        Self {
            spec: report.spec.into(),
            samples: report.samples.into_iter().map(Into::into).collect(),
            phases: report.phases.into_iter().map(Into::into).collect(),
            timeline: report.timeline.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<mobench_sdk::BenchError> for BenchError {
    fn from(err: mobench_sdk::BenchError) -> Self {
        match err {
            mobench_sdk::BenchError::Runner(runner_err) => {
                BenchError::ExecutionFailed {
                    reason: runner_err.to_string(),
                }
            }
            mobench_sdk::BenchError::UnknownFunction(name, _available) => {
                BenchError::UnknownFunction { name }
            }
            _ => BenchError::ExecutionFailed {
                reason: err.to_string(),
            },
        }
    }
}

/// Runs a benchmark by name with the given specification
///
/// This is the main FFI entry point called from mobile platforms.
#[uniffi::export]
pub fn run_benchmark(spec: BenchSpec) -> Result<BenchReport, BenchError> {
    let sdk_spec: mobench_sdk::BenchSpec = spec.into();
    let report = mobench_sdk::run_benchmark(sdk_spec)?;
    Ok(report.into())
}

// Generate UniFFI scaffolding
uniffi::setup_scaffolding!();
"#;

    let lib_rs = render_template(
        lib_rs_template,
        &[TemplateVar {
            name: "USER_CRATE",
            value: project_name.replace('-', "_"),
        }],
    );
    fs::write(crate_dir.join("src/lib.rs"), lib_rs)?;

    // Generate build.rs
    let build_rs = r#"fn main() {
    uniffi::generate_scaffolding("src/lib.rs").unwrap();
}
"#;

    fs::write(crate_dir.join("build.rs"), build_rs)?;

    // Generate uniffi-bindgen binary (used by mobench build)
    let bin_dir = crate_dir.join("src/bin");
    fs::create_dir_all(&bin_dir)?;
    let uniffi_bindgen_rs = r#"fn main() {
    uniffi::uniffi_bindgen_main()
}
"#;
    fs::write(bin_dir.join("uniffi-bindgen.rs"), uniffi_bindgen_rs)?;

    Ok(())
}

/// Generates Android project structure from templates
///
/// This function can be called standalone to generate just the Android
/// project scaffolding, useful for auto-generation during build.
///
/// # Arguments
///
/// * `output_dir` - Directory to write the `android/` project into
/// * `project_slug` - Project name (e.g., "bench-mobile" -> "bench_mobile")
/// * `default_function` - Default benchmark function to use (e.g., "bench_mobile::my_benchmark")
pub fn generate_android_project(
    output_dir: &Path,
    project_slug: &str,
    default_function: &str,
) -> Result<(), BenchError> {
    generate_android_project_with_backend(
        output_dir,
        project_slug,
        default_function,
        crate::FfiBackend::Uniffi,
    )
}

/// Generates Android project structure from templates for a specific FFI backend.
pub fn generate_android_project_with_backend(
    output_dir: &Path,
    project_slug: &str,
    default_function: &str,
    ffi_backend: crate::FfiBackend,
) -> Result<(), BenchError> {
    let android_benchmark_timeout_secs = resolve_positive_u64_env(
        "MOBENCH_ANDROID_BENCHMARK_TIMEOUT_SECS",
        DEFAULT_ANDROID_BENCHMARK_TIMEOUT_SECS,
    );
    let android_heartbeat_interval_secs = resolve_positive_u64_env(
        "MOBENCH_ANDROID_HEARTBEAT_INTERVAL_SECS",
        DEFAULT_ANDROID_HEARTBEAT_INTERVAL_SECS,
    );
    generate_android_project_with_options(
        output_dir,
        project_slug,
        default_function,
        ffi_backend,
        android_benchmark_timeout_secs,
        android_heartbeat_interval_secs,
    )
}

fn generate_android_project_with_options(
    output_dir: &Path,
    project_slug: &str,
    default_function: &str,
    ffi_backend: crate::FfiBackend,
    android_benchmark_timeout_secs: u64,
    android_heartbeat_interval_secs: u64,
) -> Result<(), BenchError> {
    let target_dir = output_dir.join("android");
    reset_generated_project_dir(&target_dir)?;
    let library_name = project_slug.replace('-', "_");
    let project_pascal = to_pascal_case(project_slug);
    // Use sanitized bundle ID component (alphanumeric only) for consistency with iOS
    // This ensures both platforms use the same naming convention: "benchmobile" not "bench-mobile"
    let package_id_component = sanitize_bundle_id_component(project_slug);
    let package_name = format!("dev.world.{}", package_id_component);
    let boltffi_kotlin_package = format!("com.mobench.{}", package_id_component);
    let vars = vec![
        TemplateVar {
            name: "PROJECT_NAME",
            value: project_slug.to_string(),
        },
        TemplateVar {
            name: "PROJECT_NAME_PASCAL",
            value: project_pascal.clone(),
        },
        TemplateVar {
            name: "APP_NAME",
            value: format!("{} Benchmark", project_pascal),
        },
        TemplateVar {
            name: "PACKAGE_NAME",
            value: package_name.clone(),
        },
        TemplateVar {
            name: "BOLTFFI_KOTLIN_PACKAGE",
            value: boltffi_kotlin_package.clone(),
        },
        TemplateVar {
            name: "UNIFFI_NAMESPACE",
            value: library_name.clone(),
        },
        TemplateVar {
            name: "LIBRARY_NAME",
            value: library_name,
        },
        TemplateVar {
            name: "DEFAULT_FUNCTION",
            value: default_function.to_string(),
        },
        TemplateVar {
            name: "ANDROID_BENCHMARK_TIMEOUT_SECS",
            value: android_benchmark_timeout_secs.to_string(),
        },
        TemplateVar {
            name: "ANDROID_HEARTBEAT_INTERVAL_SECS",
            value: android_heartbeat_interval_secs.to_string(),
        },
    ];
    render_dir(&ANDROID_TEMPLATES, &target_dir, &vars)?;

    // Move Kotlin files to the correct package directory structure
    // The package "dev.world.{project_slug}" maps to directory "dev/world/{project_slug}/"
    move_kotlin_files_to_package_dir(&target_dir, &package_name)?;
    match ffi_backend {
        crate::FfiBackend::Uniffi => {}
        crate::FfiBackend::NativeCAbi => {
            write_native_android_main_activity(&target_dir, &package_name, &vars)?;
        }
        crate::FfiBackend::BoltFfi => {
            write_boltffi_android_main_activity(&target_dir, &package_name, &vars)?;
            write_boltffi_config(
                output_dir,
                project_slug,
                "BenchRunner",
                &boltffi_kotlin_package,
            )?;
        }
    }

    Ok(())
}

fn write_native_android_main_activity(
    target_dir: &Path,
    package_name: &str,
    vars: &[TemplateVar],
) -> Result<(), BenchError> {
    let relative_package = package_name.replace('.', "/");
    let path = target_dir
        .join("app/src/main/java")
        .join(relative_package)
        .join("MainActivity.kt");
    let rendered = render_template(NATIVE_ANDROID_MAIN_ACTIVITY_TEMPLATE, vars);
    validate_no_unreplaced_placeholders(&rendered, Path::new("MainActivity.kt"))?;
    fs::write(&path, rendered).map_err(BenchError::Io)
}

fn write_boltffi_android_main_activity(
    target_dir: &Path,
    package_name: &str,
    vars: &[TemplateVar],
) -> Result<(), BenchError> {
    let relative_package = package_name.replace('.', "/");
    let path = target_dir
        .join("app/src/main/java")
        .join(relative_package)
        .join("MainActivity.kt");
    let rendered = render_template(BOLTFFI_ANDROID_MAIN_ACTIVITY_TEMPLATE, vars);
    validate_no_unreplaced_placeholders(&rendered, Path::new("MainActivity.kt"))?;
    fs::write(&path, rendered).map_err(BenchError::Io)
}

fn collect_preserved_files(
    root: &Path,
    current: &Path,
    preserved: &mut Vec<(PathBuf, Vec<u8>)>,
) -> Result<(), BenchError> {
    let mut entries = fs::read_dir(current)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(BenchError::Io)?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_preserved_files(root, &path, preserved)?;
            continue;
        }

        let relative = path.strip_prefix(root).map_err(|e| {
            BenchError::Build(format!(
                "Failed to preserve generated resource {:?}: {}",
                path, e
            ))
        })?;
        preserved.push((relative.to_path_buf(), fs::read(&path)?));
    }

    Ok(())
}

fn collect_preserved_ios_resources(
    target_dir: &Path,
) -> Result<Vec<(PathBuf, Vec<u8>)>, BenchError> {
    let resources_dir = target_dir.join("BenchRunner/BenchRunner/Resources");
    let mut preserved = Vec::new();

    if resources_dir.exists() {
        collect_preserved_files(&resources_dir, &resources_dir, &mut preserved)?;
    }

    Ok(preserved)
}

fn restore_preserved_ios_resources(
    target_dir: &Path,
    preserved_resources: &[(PathBuf, Vec<u8>)],
) -> Result<(), BenchError> {
    if preserved_resources.is_empty() {
        return Ok(());
    }

    let resources_dir = target_dir.join("BenchRunner/BenchRunner/Resources");
    for (relative, contents) in preserved_resources {
        let resource_path = resources_dir.join(relative);
        if let Some(parent) = resource_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(resource_path, contents)?;
    }

    Ok(())
}

fn reset_generated_project_dir(target_dir: &Path) -> Result<(), BenchError> {
    if target_dir.exists() {
        fs::remove_dir_all(target_dir).map_err(|e| {
            BenchError::Build(format!(
                "Failed to clear existing generated project at {:?}: {}",
                target_dir, e
            ))
        })?;
    }
    Ok(())
}

/// Moves Kotlin source files to the correct package directory structure
///
/// Android requires source files to be in directories matching their package declaration.
/// For example, a file with `package dev.world.my_project` must be in
/// `app/src/main/java/dev/world/my_project/`.
///
/// This function moves:
/// - MainActivity.kt from `app/src/main/java/` to `app/src/main/java/{package_path}/`
/// - MainActivityTest.kt from `app/src/androidTest/java/` to `app/src/androidTest/java/{package_path}/`
fn move_kotlin_files_to_package_dir(
    android_dir: &Path,
    package_name: &str,
) -> Result<(), BenchError> {
    // Convert package name to directory path (e.g., "dev.world.my_project" -> "dev/world/my_project")
    let package_path = package_name.replace('.', "/");

    // Move main source files
    let main_java_dir = android_dir.join("app/src/main/java");
    let main_package_dir = main_java_dir.join(&package_path);
    move_kotlin_file(&main_java_dir, &main_package_dir, "MainActivity.kt")?;

    // Move test source files
    let test_java_dir = android_dir.join("app/src/androidTest/java");
    let test_package_dir = test_java_dir.join(&package_path);
    move_kotlin_file(&test_java_dir, &test_package_dir, "MainActivityTest.kt")?;

    Ok(())
}

/// Moves a single Kotlin file from source directory to package directory
fn move_kotlin_file(src_dir: &Path, dest_dir: &Path, filename: &str) -> Result<(), BenchError> {
    let src_file = src_dir.join(filename);
    if !src_file.exists() {
        // File doesn't exist in source, nothing to move
        return Ok(());
    }

    // Create the package directory if it doesn't exist
    fs::create_dir_all(dest_dir).map_err(|e| {
        BenchError::Build(format!(
            "Failed to create package directory {:?}: {}",
            dest_dir, e
        ))
    })?;

    let dest_file = dest_dir.join(filename);

    // Move the file (copy + delete for cross-filesystem compatibility)
    fs::copy(&src_file, &dest_file).map_err(|e| {
        BenchError::Build(format!(
            "Failed to copy {} to {:?}: {}",
            filename, dest_file, e
        ))
    })?;

    fs::remove_file(&src_file).map_err(|e| {
        BenchError::Build(format!(
            "Failed to remove original file {:?}: {}",
            src_file, e
        ))
    })?;

    Ok(())
}

/// Generates iOS project structure from templates
///
/// This function can be called standalone to generate just the iOS
/// project scaffolding, useful for auto-generation during build.
///
/// # Arguments
///
/// * `output_dir` - Directory to write the `ios/` project into
/// * `project_slug` - Project name (e.g., "bench-mobile" -> "bench_mobile")
/// * `project_pascal` - PascalCase version of project name (e.g., "BenchMobile")
/// * `bundle_prefix` - iOS bundle ID prefix (e.g., "dev.world.bench")
/// * `default_function` - Default benchmark function to use (e.g., "bench_mobile::my_benchmark")
pub fn generate_ios_project(
    output_dir: &Path,
    project_slug: &str,
    project_pascal: &str,
    bundle_prefix: &str,
    default_function: &str,
) -> Result<(), BenchError> {
    generate_ios_project_with_backend_options(
        output_dir,
        project_slug,
        project_pascal,
        bundle_prefix,
        default_function,
        crate::FfiBackend::Uniffi,
        IosProjectOptions {
            ios_benchmark_timeout_secs: resolve_ios_benchmark_timeout_secs(
                std::env::var("MOBENCH_IOS_BENCHMARK_TIMEOUT_SECS")
                    .ok()
                    .as_deref(),
            ),
            ..IosProjectOptions::default()
        },
    )
}

/// Generates iOS project structure from templates for a specific FFI backend.
pub fn generate_ios_project_with_backend(
    output_dir: &Path,
    project_slug: &str,
    project_pascal: &str,
    bundle_prefix: &str,
    default_function: &str,
    ffi_backend: crate::FfiBackend,
) -> Result<(), BenchError> {
    generate_ios_project_with_backend_options(
        output_dir,
        project_slug,
        project_pascal,
        bundle_prefix,
        default_function,
        ffi_backend,
        IosProjectOptions {
            ios_benchmark_timeout_secs: resolve_ios_benchmark_timeout_secs(
                std::env::var("MOBENCH_IOS_BENCHMARK_TIMEOUT_SECS")
                    .ok()
                    .as_deref(),
            ),
            ..IosProjectOptions::default()
        },
    )
}

#[cfg(test)]
fn generate_ios_project_with_timeout(
    output_dir: &Path,
    project_slug: &str,
    project_pascal: &str,
    bundle_prefix: &str,
    default_function: &str,
    ios_benchmark_timeout_secs: u64,
    ffi_backend: crate::FfiBackend,
) -> Result<(), BenchError> {
    generate_ios_project_with_backend_options(
        output_dir,
        project_slug,
        project_pascal,
        bundle_prefix,
        default_function,
        ffi_backend,
        IosProjectOptions {
            ios_benchmark_timeout_secs,
            ..IosProjectOptions::default()
        },
    )
}

pub fn generate_ios_project_with_backend_options(
    output_dir: &Path,
    project_slug: &str,
    project_pascal: &str,
    bundle_prefix: &str,
    default_function: &str,
    ffi_backend: crate::FfiBackend,
    options: IosProjectOptions,
) -> Result<(), BenchError> {
    let runner = resolve_ios_runner(&options.deployment_target, Some(options.runner))?;
    let target_dir = output_dir.join("ios");
    let preserved_resources = collect_preserved_ios_resources(&target_dir)?;
    reset_generated_project_dir(&target_dir)?;
    // Sanitize bundle ID components to ensure they only contain alphanumeric characters
    // iOS bundle identifiers should not contain hyphens or underscores
    let sanitized_bundle_prefix = {
        let parts: Vec<&str> = bundle_prefix.split('.').collect();
        parts
            .iter()
            .map(|part| sanitize_bundle_id_component(part))
            .collect::<Vec<_>>()
            .join(".")
    };
    // Use the actual app name (project_pascal, e.g., "BenchRunner") for the bundle ID suffix,
    // not the crate name again. This prevents duplication like "dev.world.benchmobile.benchmobile"
    // and produces the correct "dev.world.benchmobile.BenchRunner"
    let vars = vec![
        TemplateVar {
            name: "DEFAULT_FUNCTION",
            value: default_function.to_string(),
        },
        TemplateVar {
            name: "PROJECT_NAME_PASCAL",
            value: project_pascal.to_string(),
        },
        TemplateVar {
            name: "BUNDLE_ID_PREFIX",
            value: sanitized_bundle_prefix.clone(),
        },
        TemplateVar {
            name: "BUNDLE_ID",
            value: format!("{}.{}", sanitized_bundle_prefix, project_pascal),
        },
        TemplateVar {
            name: "LIBRARY_NAME",
            value: project_slug.replace('-', "_"),
        },
        TemplateVar {
            name: "IOS_BENCHMARK_TIMEOUT_SECS",
            value: options.ios_benchmark_timeout_secs.to_string(),
        },
        TemplateVar {
            name: "IOS_DEPLOYMENT_TARGET",
            value: options.deployment_target.to_string(),
        },
        TemplateVar {
            name: "IOS_RUNNER",
            value: runner.as_str().to_string(),
        },
        TemplateVar {
            name: "BRIDGING_HEADER_IMPORTS",
            value: match ffi_backend {
                crate::FfiBackend::BoltFfi => String::new(),
                _ => format!("#import \"{}FFI.h\"", project_slug.replace('-', "_")),
            },
        },
    ];
    render_ios_dir(&IOS_TEMPLATES, &target_dir, &vars, runner)?;
    match ffi_backend {
        crate::FfiBackend::Uniffi => {}
        crate::FfiBackend::NativeCAbi => {
            write_native_ios_bench_runner_ffi(&target_dir, project_pascal, &vars)?;
        }
        crate::FfiBackend::BoltFfi => {
            write_boltffi_ios_bench_runner_ffi(&target_dir, project_pascal, &vars)?;
            write_boltffi_config(
                output_dir,
                project_slug,
                project_pascal,
                "dev.world.benchrunner",
            )?;
        }
    }
    restore_preserved_ios_resources(&target_dir, &preserved_resources)?;
    Ok(())
}

fn write_native_ios_bench_runner_ffi(
    target_dir: &Path,
    project_pascal: &str,
    vars: &[TemplateVar],
) -> Result<(), BenchError> {
    let app_dir = target_dir.join("BenchRunner").join(project_pascal);
    let path = app_dir.join("BenchRunnerFFI.swift");
    let generated_dir = app_dir.join("Generated");
    fs::create_dir_all(&generated_dir).map_err(BenchError::Io)?;
    let library_name = vars
        .iter()
        .find(|var| var.name == "LIBRARY_NAME")
        .map(|var| var.value.as_str())
        .unwrap_or("bench_mobile");
    let header_path = generated_dir.join(format!("{library_name}FFI.h"));
    fs::write(&header_path, native_c_abi_header(library_name)).map_err(BenchError::Io)?;

    let rendered = render_template(NATIVE_IOS_BENCH_RUNNER_FFI_TEMPLATE, vars);
    validate_no_unreplaced_placeholders(&rendered, Path::new("BenchRunnerFFI.swift"))?;
    fs::write(&path, rendered).map_err(BenchError::Io)
}

fn write_boltffi_ios_bench_runner_ffi(
    target_dir: &Path,
    project_pascal: &str,
    vars: &[TemplateVar],
) -> Result<(), BenchError> {
    let app_dir = target_dir.join("BenchRunner").join(project_pascal);
    let path = app_dir.join("BenchRunnerFFI.swift");
    let generated_dir = app_dir.join("Generated").join("BoltFFIGenerated");
    fs::create_dir_all(&generated_dir).map_err(BenchError::Io)?;

    let rendered = render_template(BOLTFFI_IOS_BENCH_RUNNER_FFI_TEMPLATE, vars);
    validate_no_unreplaced_placeholders(&rendered, Path::new("BenchRunnerFFI.swift"))?;
    fs::write(&path, rendered).map_err(BenchError::Io)
}

pub fn write_boltffi_config(
    project_root: &Path,
    library_name: &str,
    swift_module_name: &str,
    kotlin_package: &str,
) -> Result<(), BenchError> {
    write_boltffi_config_with_options(
        project_root,
        library_name,
        library_name,
        swift_module_name,
        kotlin_package,
        &["arm64".to_string()],
    )
}

pub fn write_boltffi_config_with_options(
    project_root: &Path,
    package_name: &str,
    crate_name: &str,
    swift_module_name: &str,
    kotlin_package: &str,
    android_architectures: &[String],
) -> Result<(), BenchError> {
    write_boltffi_config_with_paths(
        project_root,
        package_name,
        crate_name,
        swift_module_name,
        kotlin_package,
        android_architectures,
        &BoltFfiOutputPaths::default(),
    )
}

pub fn write_boltffi_config_with_output_dir(
    project_root: &Path,
    package_name: &str,
    crate_name: &str,
    swift_module_name: &str,
    kotlin_package: &str,
    android_architectures: &[String],
    output_dir: &Path,
) -> Result<(), BenchError> {
    write_boltffi_config_with_paths(
        project_root,
        package_name,
        crate_name,
        swift_module_name,
        kotlin_package,
        android_architectures,
        &BoltFfiOutputPaths::for_output_dir(output_dir),
    )
}

struct BoltFfiOutputPaths {
    android_kotlin: String,
    android_header: String,
    android_pack: String,
    apple_swift: String,
    apple_header: String,
    apple_xcframework: String,
}

impl BoltFfiOutputPaths {
    fn default() -> Self {
        Self {
            android_kotlin: "target/mobench/android/app/src/main/java".to_string(),
            android_header: "target/mobench/boltffi/android/include".to_string(),
            android_pack: "target/mobench/android/app/src/main/jniLibs".to_string(),
            apple_swift: "target/mobench/ios/BenchRunner/BenchRunner/Generated/BoltFFIGenerated"
                .to_string(),
            apple_header: "target/mobench/ios/include".to_string(),
            apple_xcframework: "target/mobench/ios".to_string(),
        }
    }

    fn for_output_dir(output_dir: &Path) -> Self {
        Self {
            android_kotlin: toml_path(output_dir.join("android/app/src/main/java")),
            android_header: toml_path(output_dir.join("boltffi/android/include")),
            android_pack: toml_path(output_dir.join("android/app/src/main/jniLibs")),
            apple_swift: toml_path(
                output_dir.join("ios/BenchRunner/BenchRunner/Generated/BoltFFIGenerated"),
            ),
            apple_header: toml_path(output_dir.join("ios/include")),
            apple_xcframework: toml_path(output_dir.join("ios")),
        }
    }
}

fn toml_path(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

fn write_boltffi_config_with_paths(
    project_root: &Path,
    package_name: &str,
    crate_name: &str,
    swift_module_name: &str,
    kotlin_package: &str,
    android_architectures: &[String],
    paths: &BoltFfiOutputPaths,
) -> Result<(), BenchError> {
    let library_name = crate_name.replace('-', "_");
    let android_architectures = android_architectures
        .iter()
        .map(|arch| format!("\"{arch}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let content = format!(
        r#"[package]
name = "{package_name}"
crate = "{crate_name}"

[targets.android]
enabled = true
architectures = [{android_architectures}]

[targets.android.kotlin]
package = "{kotlin_package}"
output = "{android_kotlin}"
library_name = "{library_name}"
desktop_loader = "system"
api_style = "top_level"

[targets.android.header]
output = "{android_header}"

[targets.android.pack]
output = "{android_pack}"

[targets.apple]
enabled = true
include_macos = false

[targets.apple.swift]
module_name = "{swift_module_name}"
output = "{apple_swift}"
ffi_module_name = "{library_name}FFI"

[targets.apple.header]
output = "{apple_header}"

[targets.apple.xcframework]
output = "{apple_xcframework}"
name = "{library_name}"

[targets.apple.spm]
layout = "split"
skip_package_swift = true
"#,
        android_kotlin = paths.android_kotlin,
        android_header = paths.android_header,
        android_pack = paths.android_pack,
        apple_swift = paths.apple_swift,
        apple_header = paths.apple_header,
        apple_xcframework = paths.apple_xcframework,
    );
    fs::write(project_root.join("boltffi.toml"), content).map_err(BenchError::Io)
}

fn native_c_abi_header(framework_name: &str) -> String {
    let guard = format!(
        "{}_MOBENCH_NATIVE_C_ABI_H",
        framework_name.to_ascii_uppercase()
    )
    .replace('-', "_");
    format!(
        r#"#ifndef {guard}
#define {guard}

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {{
#endif

typedef struct MobenchBuf {{
    uint8_t *ptr;
    uintptr_t len;
    uintptr_t cap;
}} MobenchBuf;

int32_t mobench_run_benchmark_json(const uint8_t *spec_ptr, uintptr_t spec_len, MobenchBuf *out);
void mobench_free_buf(MobenchBuf *buf);
const char *mobench_last_error_message(void);

#ifdef __cplusplus
}}
#endif

#endif
"#
    )
}

fn resolve_ios_benchmark_timeout_secs(value: Option<&str>) -> u64 {
    value
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_IOS_BENCHMARK_TIMEOUT_SECS)
}

fn resolve_positive_u64_env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(default)
}

/// Generates bench-config.toml configuration file
fn generate_config_file(output_dir: &Path, config: &InitConfig) -> Result<(), BenchError> {
    let config_target = match config.target {
        Target::Ios => "ios",
        Target::Android | Target::Both => "android",
    };
    let config_content = format!(
        r#"# mobench configuration
# This file controls how benchmarks are executed on devices.

target = "{}"
function = "example_fibonacci"
iterations = 100
warmup = 10
device_matrix = "device-matrix.yaml"
device_tags = ["default"]

[browserstack]
app_automate_username = "${{BROWSERSTACK_USERNAME}}"
app_automate_access_key = "${{BROWSERSTACK_ACCESS_KEY}}"
project = "{}-benchmarks"

[ios_xcuitest]
app = "target/ios/BenchRunner.ipa"
test_suite = "target/ios/BenchRunnerUITests.zip"
"#,
        config_target, config.project_name
    );

    fs::write(output_dir.join("bench-config.toml"), config_content)?;

    Ok(())
}

/// Generates example benchmark functions
fn generate_example_benchmarks(output_dir: &Path) -> Result<(), BenchError> {
    let examples_dir = output_dir.join("benches");
    fs::create_dir_all(&examples_dir)?;

    let example_content = r#"//! Example benchmarks
//!
//! This file demonstrates how to write benchmarks with mobench-sdk.

use mobench_sdk::benchmark;

/// Simple benchmark example
#[benchmark]
fn example_fibonacci() {
    let result = fibonacci(30);
    std::hint::black_box(result);
}

/// Another example with a loop
#[benchmark]
fn example_sum() {
    let mut sum = 0u64;
    for i in 0..10000 {
        sum = sum.wrapping_add(i);
    }
    std::hint::black_box(sum);
}

// Helper function (not benchmarked)
fn fibonacci(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => {
            let mut a = 0u64;
            let mut b = 1u64;
            for _ in 2..=n {
                let next = a.wrapping_add(b);
                a = b;
                b = next;
            }
            b
        }
    }
}
"#;

    fs::write(examples_dir.join("example.rs"), example_content)?;

    Ok(())
}

/// File extensions that should be processed for template variable substitution
const TEMPLATE_EXTENSIONS: &[&str] = &[
    "gradle",
    "xml",
    "kt",
    "java",
    "swift",
    "yml",
    "yaml",
    "json",
    "toml",
    "md",
    "txt",
    "h",
    "m",
    "plist",
    "pbxproj",
    "xcscheme",
    "xcworkspacedata",
    "entitlements",
    "modulemap",
];

fn render_dir(dir: &Dir, out_root: &Path, vars: &[TemplateVar]) -> Result<(), BenchError> {
    render_dir_filtered(dir, out_root, vars, &|_| false)
}

fn render_ios_dir(
    dir: &Dir,
    out_root: &Path,
    vars: &[TemplateVar],
    runner: IosRunner,
) -> Result<(), BenchError> {
    render_dir_filtered(dir, out_root, vars, &|path| match runner {
        IosRunner::Swiftui => {
            path == Path::new("BenchRunner/BenchRunner/UIKitLegacyRunner.swift.template")
        }
        IosRunner::UikitLegacy => {
            path == Path::new("BenchRunner/BenchRunner/BenchRunnerApp.swift.template")
                || path == Path::new("BenchRunner/BenchRunner/ContentView.swift.template")
        }
    })
}

fn render_dir_filtered(
    dir: &Dir,
    out_root: &Path,
    vars: &[TemplateVar],
    skip_file: &dyn Fn(&Path) -> bool,
) -> Result<(), BenchError> {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(sub) => {
                // Skip cache directories
                if sub.path().components().any(|c| c.as_os_str() == ".gradle") {
                    continue;
                }
                render_dir_filtered(sub, out_root, vars, skip_file)?;
            }
            DirEntry::File(file) => {
                if file.path().components().any(|c| c.as_os_str() == ".gradle") {
                    continue;
                }
                if skip_file(file.path()) {
                    continue;
                }
                // file.path() returns the full relative path from the embedded dir root
                let mut relative = file.path().to_path_buf();
                let mut contents = file.contents().to_vec();

                // Check if file has .template extension (explicit template)
                let is_explicit_template = relative
                    .extension()
                    .map(|ext| ext == "template")
                    .unwrap_or(false);

                // Check if file is a text file that should be processed for templates
                let should_render = is_explicit_template || is_template_file(&relative);

                if is_explicit_template {
                    // Remove .template extension from output filename
                    relative.set_extension("");
                }

                if should_render && let Ok(text) = std::str::from_utf8(&contents) {
                    let rendered = render_template(text, vars);
                    // Validate that all template variables were replaced
                    validate_no_unreplaced_placeholders(&rendered, &relative)?;
                    contents = rendered.into_bytes();
                }

                let out_path = out_root.join(relative);
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&out_path, contents)?;
            }
        }
    }
    Ok(())
}

/// Checks if a file should be processed for template variable substitution
/// based on its extension
fn is_template_file(path: &Path) -> bool {
    // Check for .template extension on any file
    if let Some(ext) = path.extension() {
        if ext == "template" {
            return true;
        }
        // Check if the base extension is in our list
        if let Some(ext_str) = ext.to_str() {
            return TEMPLATE_EXTENSIONS.contains(&ext_str);
        }
    }
    // Also check the filename without the .template extension
    if let Some(stem) = path.file_stem() {
        let stem_path = Path::new(stem);
        if let Some(ext) = stem_path.extension()
            && let Some(ext_str) = ext.to_str()
        {
            return TEMPLATE_EXTENSIONS.contains(&ext_str);
        }
    }
    false
}

/// Validates that no unreplaced template placeholders remain in the rendered content
fn validate_no_unreplaced_placeholders(content: &str, file_path: &Path) -> Result<(), BenchError> {
    // Find all {{...}} patterns
    let mut pos = 0;
    let mut unreplaced = Vec::new();

    while let Some(start) = content[pos..].find("{{") {
        let abs_start = pos + start;
        if let Some(end) = content[abs_start..].find("}}") {
            let placeholder = &content[abs_start..abs_start + end + 2];
            // Extract just the variable name
            let var_name = &content[abs_start + 2..abs_start + end];
            // Skip placeholders that look like Gradle variable syntax (e.g., ${...})
            // or other non-template patterns
            if !var_name.contains('$') && !var_name.contains(' ') && !var_name.is_empty() {
                unreplaced.push(placeholder.to_string());
            }
            pos = abs_start + end + 2;
        } else {
            break;
        }
    }

    if !unreplaced.is_empty() {
        return Err(BenchError::Build(format!(
            "Template validation failed for {:?}: unreplaced placeholders found: {:?}\n\n\
             This is a bug in mobench-sdk. Please report it at:\n\
             https://github.com/worldcoin/mobile-bench-rs/issues",
            file_path, unreplaced
        )));
    }

    Ok(())
}

fn render_template(input: &str, vars: &[TemplateVar]) -> String {
    let mut output = input.to_string();
    for var in vars {
        output = output.replace(&format!("{{{{{}}}}}", var.name), &var.value);
    }
    output
}

/// Sanitizes a string to be a valid iOS bundle identifier component
///
/// Bundle identifiers can only contain alphanumeric characters (A-Z, a-z, 0-9),
/// hyphens (-), and dots (.). However, to avoid issues and maintain consistency,
/// this function converts all non-alphanumeric characters to lowercase letters only.
///
/// Examples:
/// - "bench-mobile" -> "benchmobile"
/// - "bench_mobile" -> "benchmobile"
/// - "my-project_name" -> "myprojectname"
pub fn sanitize_bundle_id_component(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn sanitize_package_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .replace("--", "-")
}

/// Converts a string to PascalCase
pub fn to_pascal_case(input: &str) -> String {
    input
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            let first = chars.next().unwrap().to_ascii_uppercase();
            let rest: String = chars.map(|c| c.to_ascii_lowercase()).collect();
            format!("{}{}", first, rest)
        })
        .collect::<String>()
}

/// Checks if the Android project scaffolding exists at the given output directory
///
/// Returns true if the `android/build.gradle` or `android/build.gradle.kts` file exists.
pub fn android_project_exists(output_dir: &Path) -> bool {
    let android_dir = output_dir.join("android");
    android_dir.join("build.gradle").exists() || android_dir.join("build.gradle.kts").exists()
}

/// Checks if the iOS project scaffolding exists at the given output directory
///
/// Returns true if the `ios/BenchRunner/project.yml` file exists.
pub fn ios_project_exists(output_dir: &Path) -> bool {
    output_dir.join("ios/BenchRunner/project.yml").exists()
}

/// Checks whether an existing iOS project was generated for the given library name.
///
/// Returns `false` if the xcframework reference in `project.yml` doesn't match,
/// which means the project needs to be regenerated for the new crate.
fn ios_project_matches_library(output_dir: &Path, library_name: &str) -> bool {
    let project_yml = output_dir.join("ios/BenchRunner/project.yml");
    let Ok(content) = std::fs::read_to_string(&project_yml) else {
        return false;
    };
    let expected = format!("../{}.xcframework", library_name);
    content.contains(&expected)
}

/// Checks whether an existing Android project was generated for the given library name.
///
/// Returns `false` if the JNI library name in `build.gradle` doesn't match,
/// which means the project needs to be regenerated for the new crate.
fn android_project_matches_library(output_dir: &Path, library_name: &str) -> bool {
    let build_gradle = output_dir.join("android/app/build.gradle");
    let Ok(content) = std::fs::read_to_string(&build_gradle) else {
        return false;
    };
    let expected = format!("lib{}.so", library_name);
    content.contains(&expected)
}

/// Checks whether an existing Android runner was generated for the selected FFI backend.
fn android_project_matches_backend(output_dir: &Path, ffi_backend: crate::FfiBackend) -> bool {
    let Some(main_activity) = read_android_main_activity(output_dir) else {
        return false;
    };

    match ffi_backend {
        crate::FfiBackend::Uniffi => {
            main_activity.contains("uniffi.") || main_activity.contains("runBenchmark(spec")
        }
        crate::FfiBackend::NativeCAbi => {
            main_activity.contains("mobench_run_benchmark_json")
                && main_activity.contains("com.sun.jna")
        }
        crate::FfiBackend::BoltFfi => main_activity.contains("runBenchmarkJson(specJson"),
    }
}

fn read_android_main_activity(output_dir: &Path) -> Option<String> {
    let java_root = output_dir.join("android/app/src/main/java");
    let mut stack = vec![java_root];
    let mut content = String::new();

    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|name| name.to_str()) == Some("MainActivity.kt") {
                let file_content = std::fs::read_to_string(path).ok()?;
                content.push_str(&file_content);
                content.push('\n');
            }
        }
    }

    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}

/// Detects the first benchmark function in a crate by scanning src/lib.rs for `#[benchmark]`
///
/// This function looks for functions marked with the `#[benchmark]` attribute and returns
/// the first one found in the format `{crate_name}::{function_name}`.
///
/// # Arguments
///
/// * `crate_dir` - Path to the crate directory containing Cargo.toml
/// * `crate_name` - Name of the crate (used as prefix for the function name)
///
/// # Returns
///
/// * `Some(String)` - The detected function name in format `crate_name::function_name`
/// * `None` - If no benchmark functions are found or if the file cannot be read
pub fn detect_default_function(crate_dir: &Path, crate_name: &str) -> Option<String> {
    let lib_rs = crate_dir.join("src/lib.rs");
    if !lib_rs.exists() {
        return None;
    }

    let file = fs::File::open(&lib_rs).ok()?;
    let reader = BufReader::new(file);

    let mut found_benchmark_attr = false;
    let crate_name_normalized = crate_name.replace('-', "_");

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();

        // Check for #[benchmark] attribute
        if trimmed == "#[benchmark]" || trimmed.starts_with("#[benchmark(") {
            found_benchmark_attr = true;
            continue;
        }

        // If we found a benchmark attribute, look for the function definition
        if found_benchmark_attr {
            // Look for "fn function_name" or "pub fn function_name"
            if let Some(fn_pos) = trimmed.find("fn ") {
                let after_fn = &trimmed[fn_pos + 3..];
                // Extract function name (until '(' or whitespace)
                let fn_name: String = after_fn
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();

                if !fn_name.is_empty() {
                    return Some(format!("{}::{}", crate_name_normalized, fn_name));
                }
            }
            // Reset if we hit a line that's not a function definition
            // (could be another attribute or comment)
            if !trimmed.starts_with('#') && !trimmed.starts_with("//") && !trimmed.is_empty() {
                found_benchmark_attr = false;
            }
        }
    }

    None
}

/// Detects all benchmark functions in a crate by scanning src/lib.rs for `#[benchmark]`
///
/// This function looks for functions marked with the `#[benchmark]` attribute and returns
/// all found in the format `{crate_name}::{function_name}`.
///
/// # Arguments
///
/// * `crate_dir` - Path to the crate directory containing Cargo.toml
/// * `crate_name` - Name of the crate (used as prefix for the function names)
///
/// # Returns
///
/// A vector of benchmark function names in format `crate_name::function_name`
pub fn detect_all_benchmarks(crate_dir: &Path, crate_name: &str) -> Vec<String> {
    let lib_rs = crate_dir.join("src/lib.rs");
    if !lib_rs.exists() {
        return Vec::new();
    }

    let Ok(file) = fs::File::open(&lib_rs) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);

    let mut benchmarks = Vec::new();
    let mut found_benchmark_attr = false;
    let crate_name_normalized = crate_name.replace('-', "_");

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();

        // Check for #[benchmark] attribute
        if trimmed == "#[benchmark]" || trimmed.starts_with("#[benchmark(") {
            found_benchmark_attr = true;
            continue;
        }

        // If we found a benchmark attribute, look for the function definition
        if found_benchmark_attr {
            // Look for "fn function_name" or "pub fn function_name"
            if let Some(fn_pos) = trimmed.find("fn ") {
                let after_fn = &trimmed[fn_pos + 3..];
                // Extract function name (until '(' or whitespace)
                let fn_name: String = after_fn
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();

                if !fn_name.is_empty() {
                    benchmarks.push(format!("{}::{}", crate_name_normalized, fn_name));
                }
                found_benchmark_attr = false;
            }
            // Reset if we hit a line that's not a function definition
            // (could be another attribute or comment)
            if !trimmed.starts_with('#') && !trimmed.starts_with("//") && !trimmed.is_empty() {
                found_benchmark_attr = false;
            }
        }
    }

    benchmarks
}

/// Validates that a benchmark function exists in the crate source
///
/// # Arguments
///
/// * `crate_dir` - Path to the crate directory containing Cargo.toml
/// * `crate_name` - Name of the crate (used as prefix for the function names)
/// * `function_name` - The function name to validate (with or without crate prefix)
///
/// # Returns
///
/// `true` if the function is found, `false` otherwise
pub fn validate_benchmark_exists(crate_dir: &Path, crate_name: &str, function_name: &str) -> bool {
    let benchmarks = detect_all_benchmarks(crate_dir, crate_name);
    let crate_name_normalized = crate_name.replace('-', "_");

    // Normalize the function name - add crate prefix if missing
    let normalized_name = if function_name.contains("::") {
        function_name.to_string()
    } else {
        format!("{}::{}", crate_name_normalized, function_name)
    };

    benchmarks.iter().any(|b| b == &normalized_name)
}

/// Resolves the default benchmark function for a project
///
/// This function attempts to auto-detect benchmark functions from the crate's source.
/// If no benchmarks are found, it falls back to a sensible default based on the crate name.
///
/// # Arguments
///
/// * `project_root` - Root directory of the project
/// * `crate_name` - Name of the benchmark crate
/// * `crate_dir` - Optional explicit crate directory (if None, will search standard locations)
///
/// # Returns
///
/// The default function name in format `crate_name::function_name`
pub fn resolve_default_function(
    project_root: &Path,
    crate_name: &str,
    crate_dir: Option<&Path>,
) -> String {
    let crate_name_normalized = crate_name.replace('-', "_");

    // Try to find the crate directory
    let search_dirs: Vec<PathBuf> = if let Some(dir) = crate_dir {
        vec![dir.to_path_buf()]
    } else {
        vec![
            project_root.join("bench-mobile"),
            project_root.join("crates").join(crate_name),
            project_root.to_path_buf(),
        ]
    };

    // Try to detect benchmarks from each potential location
    for dir in &search_dirs {
        if dir.join("Cargo.toml").exists()
            && let Some(detected) = detect_default_function(dir, &crate_name_normalized)
        {
            return detected;
        }
    }

    // Fallback: use a sensible default based on crate name
    format!("{}::example_benchmark", crate_name_normalized)
}

/// Auto-generates Android project scaffolding from a crate name
///
/// This is a convenience function that derives template variables from the
/// crate name and generates the Android project structure. It auto-detects
/// the default benchmark function from the crate's source code.
///
/// # Arguments
///
/// * `output_dir` - Directory to write the `android/` project into
/// * `crate_name` - Name of the benchmark crate (e.g., "bench-mobile")
pub fn ensure_android_project(output_dir: &Path, crate_name: &str) -> Result<(), BenchError> {
    ensure_android_project_with_options(output_dir, crate_name, None, None)
}

/// Auto-generates Android project scaffolding with additional options
///
/// This is a more flexible version of `ensure_android_project` that allows
/// specifying a custom default function and/or crate directory.
///
/// # Arguments
///
/// * `output_dir` - Directory to write the `android/` project into
/// * `crate_name` - Name of the benchmark crate (e.g., "bench-mobile")
/// * `project_root` - Optional project root for auto-detecting benchmarks (defaults to output_dir parent)
/// * `crate_dir` - Optional explicit crate directory for benchmark detection
pub fn ensure_android_project_with_options(
    output_dir: &Path,
    crate_name: &str,
    project_root: Option<&Path>,
    crate_dir: Option<&Path>,
) -> Result<(), BenchError> {
    ensure_android_project_with_backend_options(
        output_dir,
        crate_name,
        project_root,
        crate_dir,
        crate::FfiBackend::Uniffi,
    )
}

/// Auto-generates Android project scaffolding for the selected FFI backend.
pub fn ensure_android_project_with_backend_options(
    output_dir: &Path,
    crate_name: &str,
    project_root: Option<&Path>,
    crate_dir: Option<&Path>,
    ffi_backend: crate::FfiBackend,
) -> Result<(), BenchError> {
    let library_name = crate_name.replace('-', "_");
    let project_exists = android_project_exists(output_dir);
    let project_matches_library = android_project_matches_library(output_dir, &library_name);
    let project_matches_backend = android_project_matches_backend(output_dir, ffi_backend);

    if project_exists && !project_matches_library {
        println!(
            "Existing Android scaffolding does not match library, regenerating for {} backend...",
            ffi_backend
        );
    } else if project_exists && !project_matches_backend {
        println!(
            "Existing Android scaffolding does not match FFI backend, regenerating for {} backend...",
            ffi_backend
        );
    } else if project_exists {
        println!(
            "Refreshing generated Android scaffolding for {} backend...",
            ffi_backend
        );
    } else {
        println!(
            "Android project not found, generating scaffolding for {} backend...",
            ffi_backend
        );
    }

    let project_slug = crate_name.replace('-', "_");

    // Resolve the default function by auto-detecting from source
    let effective_root = project_root.unwrap_or_else(|| output_dir.parent().unwrap_or(output_dir));
    let default_function = resolve_default_function(effective_root, crate_name, crate_dir);

    generate_android_project_with_backend(
        output_dir,
        &project_slug,
        &default_function,
        ffi_backend,
    )?;
    println!(
        "  Generated Android project at {:?}",
        output_dir.join("android")
    );
    println!("  Default benchmark function: {}", default_function);
    Ok(())
}

/// Auto-generates iOS project scaffolding from a crate name
///
/// This is a convenience function that derives template variables from the
/// crate name and generates the iOS project structure. It auto-detects
/// the default benchmark function from the crate's source code.
///
/// # Arguments
///
/// * `output_dir` - Directory to write the `ios/` project into
/// * `crate_name` - Name of the benchmark crate (e.g., "bench-mobile")
pub fn ensure_ios_project(output_dir: &Path, crate_name: &str) -> Result<(), BenchError> {
    ensure_ios_project_with_options(output_dir, crate_name, None, None)
}

/// Auto-generates iOS project scaffolding with additional options
///
/// This is a more flexible version of `ensure_ios_project` that allows
/// specifying a custom default function and/or crate directory.
///
/// # Arguments
///
/// * `output_dir` - Directory to write the `ios/` project into
/// * `crate_name` - Name of the benchmark crate (e.g., "bench-mobile")
/// * `project_root` - Optional project root for auto-detecting benchmarks (defaults to output_dir parent)
/// * `crate_dir` - Optional explicit crate directory for benchmark detection
pub fn ensure_ios_project_with_options(
    output_dir: &Path,
    crate_name: &str,
    project_root: Option<&Path>,
    crate_dir: Option<&Path>,
) -> Result<(), BenchError> {
    ensure_ios_project_with_backend_options(
        output_dir,
        crate_name,
        project_root,
        crate_dir,
        crate::FfiBackend::Uniffi,
        IosProjectOptions::default(),
    )
}

/// Auto-generates iOS project scaffolding for the selected FFI backend.
pub fn ensure_ios_project_with_backend_options(
    output_dir: &Path,
    crate_name: &str,
    project_root: Option<&Path>,
    crate_dir: Option<&Path>,
    ffi_backend: crate::FfiBackend,
    options: IosProjectOptions,
) -> Result<(), BenchError> {
    let library_name = crate_name.replace('-', "_");
    let project_exists = ios_project_exists(output_dir);
    let project_matches = ios_project_matches_library(output_dir, &library_name);
    if project_exists && !project_matches {
        println!(
            "Existing iOS scaffolding does not match library, regenerating for {} backend...",
            ffi_backend
        );
    } else if project_exists {
        println!(
            "Refreshing generated iOS scaffolding for {} backend...",
            ffi_backend
        );
    } else {
        println!(
            "iOS project not found, generating scaffolding for {} backend...",
            ffi_backend
        );
    }

    // Use fixed "BenchRunner" for project/scheme name to match template directory structure
    let project_pascal = "BenchRunner";
    // Derive library name and bundle prefix from crate name
    let library_name = crate_name.replace('-', "_");
    // Use sanitized bundle ID component (alphanumeric only) to avoid iOS validation issues
    // e.g., "bench-mobile" or "bench_mobile" -> "benchmobile"
    let bundle_id_component = sanitize_bundle_id_component(crate_name);
    let bundle_prefix = format!("dev.world.{}", bundle_id_component);

    // Resolve the default function by auto-detecting from source
    let effective_root = project_root.unwrap_or_else(|| output_dir.parent().unwrap_or(output_dir));
    let default_function = resolve_default_function(effective_root, crate_name, crate_dir);

    generate_ios_project_with_backend_options(
        output_dir,
        &library_name,
        project_pascal,
        &bundle_prefix,
        &default_function,
        ffi_backend,
        options,
    )?;
    println!("  Generated iOS project at {:?}", output_dir.join("ios"));
    println!("  Default benchmark function: {}", default_function);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_generate_bench_mobile_crate() {
        let temp_dir = env::temp_dir().join("mobench-sdk-test");
        fs::create_dir_all(&temp_dir).unwrap();

        let result = generate_bench_mobile_crate(&temp_dir, "test_project");
        assert!(result.is_ok());

        // Verify files were created
        assert!(temp_dir.join("bench-mobile/Cargo.toml").exists());
        assert!(temp_dir.join("bench-mobile/src/lib.rs").exists());
        assert!(temp_dir.join("bench-mobile/build.rs").exists());
        let cargo_toml =
            fs::read_to_string(temp_dir.join("bench-mobile/Cargo.toml")).expect("read Cargo.toml");
        assert!(
            cargo_toml.contains(
                r#"mobench-sdk = { path = "..", default-features = false, features = ["registry"] }"#
            ),
            "generated FFI wrapper should depend on the narrow registry feature, got:\n{cargo_toml}"
        );

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_generate_android_project_no_unreplaced_placeholders() {
        let temp_dir = env::temp_dir().join("mobench-sdk-android-test");
        // Clean up any previous test run
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let result =
            generate_android_project(&temp_dir, "my-bench-project", "my_bench_project::test_func");
        assert!(
            result.is_ok(),
            "generate_android_project failed: {:?}",
            result.err()
        );

        // Verify key files exist
        let android_dir = temp_dir.join("android");
        assert!(android_dir.join("settings.gradle").exists());
        assert!(android_dir.join("app/build.gradle").exists());
        assert!(
            android_dir
                .join("app/src/main/AndroidManifest.xml")
                .exists()
        );
        assert!(
            android_dir
                .join("app/src/main/res/values/strings.xml")
                .exists()
        );
        assert!(
            android_dir
                .join("app/src/main/res/values/themes.xml")
                .exists()
        );

        // Verify no unreplaced placeholders remain in generated files
        let files_to_check = [
            "settings.gradle",
            "app/build.gradle",
            "app/src/main/AndroidManifest.xml",
            "app/src/main/res/values/strings.xml",
            "app/src/main/res/values/themes.xml",
        ];

        for file in files_to_check {
            let path = android_dir.join(file);
            let contents =
                fs::read_to_string(&path).unwrap_or_else(|_| panic!("Failed to read {}", file));

            // Check for unreplaced placeholders
            let has_placeholder = contents.contains("{{") && contents.contains("}}");
            assert!(
                !has_placeholder,
                "File {} contains unreplaced template placeholders: {}",
                file, contents
            );
        }

        // Verify specific substitutions were made
        let settings = fs::read_to_string(android_dir.join("settings.gradle")).unwrap();
        assert!(
            settings.contains("my-bench-project-android")
                || settings.contains("my_bench_project-android"),
            "settings.gradle should contain project name"
        );

        let build_gradle = fs::read_to_string(android_dir.join("app/build.gradle")).unwrap();
        // Package name should be sanitized (no hyphens/underscores) for consistency with iOS
        assert!(
            build_gradle.contains("dev.world.mybenchproject"),
            "build.gradle should contain sanitized package name 'dev.world.mybenchproject'"
        );
        assert!(
            !build_gradle.contains("testBuildType \"release\""),
            "debug builds should be able to produce assembleDebugAndroidTest"
        );
        assert!(
            build_gradle.contains("mobenchTestBuildType"),
            "release builds should be able to request assembleReleaseAndroidTest"
        );

        let manifest =
            fs::read_to_string(android_dir.join("app/src/main/AndroidManifest.xml")).unwrap();
        assert!(
            manifest.contains("Theme.MyBenchProject"),
            "AndroidManifest.xml should contain PascalCase theme name"
        );

        let strings =
            fs::read_to_string(android_dir.join("app/src/main/res/values/strings.xml")).unwrap();
        assert!(
            strings.contains("Benchmark"),
            "strings.xml should contain app name with Benchmark"
        );

        // Verify Kotlin files are in the correct package directory structure
        // For package "dev.world.mybenchproject", files should be in "dev/world/mybenchproject/"
        let main_activity_path =
            android_dir.join("app/src/main/java/dev/world/mybenchproject/MainActivity.kt");
        assert!(
            main_activity_path.exists(),
            "MainActivity.kt should be in package directory: {:?}",
            main_activity_path
        );

        let test_activity_path = android_dir
            .join("app/src/androidTest/java/dev/world/mybenchproject/MainActivityTest.kt");
        assert!(
            test_activity_path.exists(),
            "MainActivityTest.kt should be in package directory: {:?}",
            test_activity_path
        );

        // Verify the files are NOT in the root java directory
        assert!(
            !android_dir
                .join("app/src/main/java/MainActivity.kt")
                .exists(),
            "MainActivity.kt should not be in root java directory"
        );
        assert!(
            !android_dir
                .join("app/src/androidTest/java/MainActivityTest.kt")
                .exists(),
            "MainActivityTest.kt should not be in root java directory"
        );

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_generate_android_project_replaces_previous_package_tree() {
        let temp_dir = env::temp_dir().join("mobench-sdk-android-regenerate-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        generate_android_project(&temp_dir, "ffi_benchmark", "ffi_benchmark::bench_fibonacci")
            .unwrap();
        let old_package_dir = temp_dir.join("android/app/src/main/java/dev/world/ffibenchmark");
        assert!(
            old_package_dir.exists(),
            "expected first package tree to exist"
        );

        generate_android_project(
            &temp_dir,
            "basic_benchmark",
            "basic_benchmark::bench_fibonacci",
        )
        .unwrap();

        let new_package_dir = temp_dir.join("android/app/src/main/java/dev/world/basicbenchmark");
        assert!(
            new_package_dir.exists(),
            "expected new package tree to exist"
        );
        assert!(
            !old_package_dir.exists(),
            "old package tree should be removed when regenerating the Android scaffold"
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_generate_android_native_backend_runner_template() {
        let temp_dir = env::temp_dir().join("mobench-sdk-android-native-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        generate_android_project_with_backend(
            &temp_dir,
            "native_benchmark",
            "native_benchmark::bench_prove",
            crate::FfiBackend::NativeCAbi,
        )
        .unwrap();

        let main_activity = fs::read_to_string(
            temp_dir.join("android/app/src/main/java/dev/world/nativebenchmark/MainActivity.kt"),
        )
        .unwrap();
        assert!(main_activity.contains("com.sun.jna.Native"));
        assert!(main_activity.contains("mobench_run_benchmark_json"));
        assert!(main_activity.contains("BENCH_JSON"));
        assert!(main_activity.contains("bench_spec.json"));
        assert!(
            !main_activity.contains("uniffi."),
            "native Android runner must not import UniFFI bindings:\n{}",
            main_activity
        );
        assert!(
            !main_activity.contains("runBenchmark("),
            "native Android runner must call the JSON C ABI, not UniFFI runBenchmark"
        );
        let package_manifest =
            fs::read_to_string(temp_dir.join("android/app/build.gradle")).unwrap();
        assert!(package_manifest.contains("net.java.dev.jna:jna"));
        assert!(!package_manifest.contains("uniffi"));
        let android_manifest =
            fs::read_to_string(temp_dir.join("android/app/src/main/AndroidManifest.xml")).unwrap();
        assert!(android_manifest.contains("android:name=\".MainActivity\""));
        assert!(!android_manifest.contains("uniffi"));
        assert!(!android_manifest.contains("{{"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_generate_android_boltffi_backend_runner_template() {
        let temp_dir = env::temp_dir().join("mobench-sdk-android-boltffi-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        generate_android_project_with_backend(
            &temp_dir,
            "bolt_benchmark",
            "bolt_benchmark::bench_prove",
            crate::FfiBackend::BoltFfi,
        )
        .unwrap();

        let main_activity = fs::read_to_string(
            temp_dir.join("android/app/src/main/java/dev/world/boltbenchmark/MainActivity.kt"),
        )
        .unwrap();
        assert!(main_activity.contains("import com.mobench.boltbenchmark.runBenchmarkJson"));
        assert!(main_activity.contains("runBenchmarkJson(specJson = spec.toString())"));
        assert!(main_activity.contains("fun isBenchmarkComplete()"));
        assert!(main_activity.contains("BENCH_JSON"));
        assert!(
            !main_activity.contains("uniffi."),
            "BoltFFI Android runner must not import UniFFI bindings:\n{}",
            main_activity
        );
        assert!(
            !main_activity.contains("com.sun.jna"),
            "BoltFFI Android runner must not use the native C ABI JNA bridge"
        );

        let boltffi_toml = fs::read_to_string(temp_dir.join("boltffi.toml")).unwrap();
        assert!(boltffi_toml.contains("name = \"bolt_benchmark\""));
        assert!(boltffi_toml.contains("crate = \"bolt_benchmark\""));
        assert!(boltffi_toml.contains("package = \"com.mobench.boltbenchmark\""));
        assert!(boltffi_toml.contains("desktop_loader = \"system\""));
        assert!(boltffi_toml.contains("output = \"target/mobench/android/app/src/main/java\""));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_ensure_android_project_regenerates_when_ffi_backend_changes() {
        let temp_dir = env::temp_dir().join("mobench-sdk-android-backend-switch-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        generate_android_project_with_backend(
            &temp_dir,
            "switch_benchmark",
            "switch_benchmark::bench_prove",
            crate::FfiBackend::Uniffi,
        )
        .unwrap();

        let main_activity_path =
            temp_dir.join("android/app/src/main/java/dev/world/switchbenchmark/MainActivity.kt");
        let uniffi_main = fs::read_to_string(&main_activity_path).unwrap();
        assert!(uniffi_main.contains("uniffi."));

        ensure_android_project_with_backend_options(
            &temp_dir,
            "switch_benchmark",
            None,
            None,
            crate::FfiBackend::BoltFfi,
        )
        .unwrap();

        let boltffi_main = fs::read_to_string(&main_activity_path).unwrap();
        assert!(boltffi_main.contains("runBenchmarkJson(specJson = spec.toString())"));
        assert!(
            !boltffi_main.contains("uniffi."),
            "backend changes should regenerate the Android runner:\n{}",
            boltffi_main
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_ensure_android_project_refreshes_existing_backend_scaffolding() {
        let temp_dir = env::temp_dir().join("mobench-sdk-android-refresh-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        generate_android_project_with_backend(
            &temp_dir,
            "refresh_benchmark",
            "refresh_benchmark::bench_prove",
            crate::FfiBackend::BoltFfi,
        )
        .unwrap();

        let main_activity_path =
            temp_dir.join("android/app/src/main/java/dev/world/refreshbenchmark/MainActivity.kt");
        let stale_main = fs::read_to_string(&main_activity_path)
            .unwrap()
            .replace("BENCH_JSON_START", "STALE_BENCH_JSON_START");
        fs::write(&main_activity_path, stale_main).unwrap();

        ensure_android_project_with_backend_options(
            &temp_dir,
            "refresh_benchmark",
            None,
            None,
            crate::FfiBackend::BoltFfi,
        )
        .unwrap();

        let refreshed_main = fs::read_to_string(&main_activity_path).unwrap();
        assert!(refreshed_main.contains("BENCH_JSON_START"));
        assert!(
            !refreshed_main.contains("STALE_BENCH_JSON_START"),
            "existing same-backend scaffolding should be refreshed:\n{}",
            refreshed_main
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_write_boltffi_config_can_target_explicit_output_dir() {
        let temp_dir = env::temp_dir().join("mobench-sdk-boltffi-config-output-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let output_dir = temp_dir.join("custom-mobench-output");

        write_boltffi_config_with_output_dir(
            &temp_dir,
            "bolt_benchmark",
            "bolt-benchmark",
            "BenchRunner",
            "com.mobench.boltbenchmark",
            &["arm64".to_string(), "x86_64".to_string()],
            &output_dir,
        )
        .unwrap();

        let boltffi_toml = fs::read_to_string(temp_dir.join("boltffi.toml")).unwrap();
        assert!(boltffi_toml.contains(&format!(
            "output = \"{}\"",
            output_dir
                .join("android/app/src/main/java")
                .to_string_lossy()
        )));
        assert!(boltffi_toml.contains(&format!(
            "output = \"{}\"",
            output_dir
                .join("ios/BenchRunner/BenchRunner/Generated/BoltFFIGenerated")
                .to_string_lossy()
        )));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_is_template_file() {
        assert!(is_template_file(Path::new("settings.gradle")));
        assert!(is_template_file(Path::new("app/build.gradle")));
        assert!(is_template_file(Path::new("AndroidManifest.xml")));
        assert!(is_template_file(Path::new("strings.xml")));
        assert!(is_template_file(Path::new("MainActivity.kt.template")));
        assert!(is_template_file(Path::new("project.yml")));
        assert!(is_template_file(Path::new("Info.plist")));
        assert!(!is_template_file(Path::new("libfoo.so")));
        assert!(!is_template_file(Path::new("image.png")));
    }

    #[test]
    fn test_mobile_templates_read_process_peak_memory_compatibly() {
        let android =
            include_str!("../templates/android/app/src/main/java/MainActivity.kt.template");
        assert!(
            !android.contains("sample.processPeakMemoryKb"),
            "Android template should not require generated bindings to expose processPeakMemoryKb"
        );
        assert!(
            !android.contains("it.processPeakMemoryKb"),
            "Android template should not require generated bindings to expose processPeakMemoryKb"
        );
        assert!(android.contains("optionalProcessPeakMemoryKb(sample)"));
        assert!(
            !android.contains("sample.cpuTimeMs"),
            "Android template should tolerate BenchSample without cpuTimeMs"
        );
        assert!(
            !android.contains("sample.peakMemoryKb"),
            "Android template should tolerate BenchSample without peakMemoryKb"
        );
        assert!(
            !android.contains("report.phases"),
            "Android template should tolerate BenchReport without phases"
        );
        assert!(android.contains("ProcessMemorySampler"));
        assert!(android.contains("sampleIntervalMs: Long = 1000L"));
        assert!(android.contains("/proc/self/smaps_rollup"));
        assert!(android.contains("class BenchmarkWorkerService : Service()"));
        assert!(android.contains("ResultReceiver(Handler(Looper.getMainLooper()))"));
        assert!(android.contains("startForegroundService(intent)"));
        assert!(android.contains("startForeground(FOREGROUND_NOTIFICATION_ID"));
        assert!(android.contains("fun isBenchmarkComplete()"));
        assert!(!android.contains("resultLatch.await"));
        assert!(android.contains("memory_process\", \"isolated_worker\""));

        let android_test = include_str!(
            "../templates/android/app/src/androidTest/java/MainActivityTest.kt.template"
        );
        assert!(android_test.contains("Log.i(\"BenchRunnerTest\""));
        assert!(android_test.contains("Thread.sleep(pollMs)"));
        assert!(
            android_test.contains("TimeUnit.SECONDS.toMillis({{ANDROID_HEARTBEAT_INTERVAL_SECS}})")
        );
        assert!(
            android_test.contains("TimeUnit.SECONDS.toMillis({{ANDROID_BENCHMARK_TIMEOUT_SECS}})")
        );
        assert!(android_test.contains("activity.isBenchmarkComplete()"));

        let ios_test = include_str!(
            "../templates/ios/BenchRunner/BenchRunnerUITests/BenchRunnerUITests.swift.template"
        );
        assert!(
            ios_test.contains("\\\"error\\\""),
            "iOS XCUITest template should fail when the benchmark report is an error payload"
        );

        let android_manifest =
            include_str!("../templates/android/app/src/main/AndroidManifest.xml");
        assert!(android_manifest.contains("android.permission.FOREGROUND_SERVICE"));
        assert!(android_manifest.contains("android.permission.FOREGROUND_SERVICE_DATA_SYNC"));
        assert!(android_manifest.contains("android:name=\".BenchmarkWorkerService\""));
        assert!(android_manifest.contains("android:foregroundServiceType=\"dataSync\""));
        assert!(android_manifest.contains("android:process=\":mobench_worker\""));

        let android_build_gradle = include_str!("../templates/android/app/build.gradle");
        assert!(android_build_gradle.contains("generatedMainBenchSpec"));
        assert!(android_build_gradle.contains("if (!generatedMainBenchSpec.exists())"));

        let ios =
            include_str!("../templates/ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift.template");
        assert!(
            !ios.contains("sample.processPeakMemoryKb"),
            "iOS template should not require generated bindings to expose processPeakMemoryKb"
        );
        assert!(
            !ios.contains(r"\.processPeakMemoryKb"),
            "iOS template should not require generated bindings to expose processPeakMemoryKb"
        );
        assert!(ios.contains("optionalProcessPeakMemoryKb(sample)"));
        assert!(ios.contains("return [\n                \"name\": name,"));
        assert!(
            !ios.contains("sample.cpuTimeMs"),
            "iOS template should tolerate BenchSample without cpuTimeMs"
        );
        assert!(
            !ios.contains("sample.peakMemoryKb"),
            "iOS template should tolerate BenchSample without peakMemoryKb"
        );
        assert!(
            !ios.contains("report.phases"),
            "iOS template should tolerate BenchReport without phases"
        );
        assert!(ios.contains("compactMap { optionalProcessPeakMemoryKb($0) }"));
        assert!(ios.contains("ProcessMemorySampler"));
        assert!(ios.contains("currentProcessResidentMemoryKb"));
        assert!(ios.contains("task_info("));
        assert!(ios.contains("\"memory_process\": \"benchmark_app\""));
        assert!(ios.contains("generateJSONReport(report, runProcessPeakMemoryKb:"));
        assert!(ios.contains("processPeakSamplesKb.max() ?? runProcessPeakMemoryKb"));
    }

    #[test]
    fn test_android_worker_result_codes_do_not_collide_with_activity_constants() {
        let generated_runner =
            include_str!("../templates/android/app/src/main/java/MainActivity.kt.template");
        let native_runner = include_str!("native_templates/android/MainActivity.kt.template");

        for (runner, success_receiver) in [
            (generated_runner, "MobenchResultCode.SUCCESS ->"),
            (native_runner, "resultCode == MobenchResultCode.SUCCESS"),
        ] {
            assert!(
                runner.contains("private object MobenchResultCode"),
                "worker result codes must live behind a qualified namespace"
            );
            assert!(runner.contains(success_receiver));
            assert!(runner.contains(
                "if (result.errorMessage == null) MobenchResultCode.SUCCESS else MobenchResultCode.ERROR"
            ));
            assert!(
                !runner.contains("private const val RESULT_OK"),
                "Activity.RESULT_OK shadows an unqualified top-level RESULT_OK"
            );
        }
        assert!(generated_runner.contains("MobenchResultCode.HEARTBEAT ->"));
        assert!(generated_runner.contains("receiver?.send(MobenchResultCode.HEARTBEAT"));
    }

    #[test]
    fn test_android_runners_emit_collectable_chunked_benchmark_reports() {
        let generated_runner =
            include_str!("../templates/android/app/src/main/java/MainActivity.kt.template");
        let native_runner = include_str!("native_templates/android/MainActivity.kt.template");
        let boltffi_runner = include_str!("boltffi_templates/android/MainActivity.kt.template");

        for runner in [generated_runner, native_runner, boltffi_runner] {
            assert!(runner.contains("BENCH_JSON_START"));
            assert!(runner.contains("BENCH_JSON_CHUNK"));
            assert!(runner.contains("BENCH_JSON_END"));
            assert!(runner.contains("val chunkSize = 1000"));
            assert!(runner.contains("Character.isHighSurrogate"));
        }
    }

    #[test]
    fn generated_android_test_uses_configured_timeout_and_heartbeat() {
        let temp_dir = env::temp_dir().join("mobench-sdk-android-timeout-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create temp dir");

        unsafe {
            std::env::set_var("MOBENCH_ANDROID_BENCHMARK_TIMEOUT_SECS", "7200");
            std::env::set_var("MOBENCH_ANDROID_HEARTBEAT_INTERVAL_SECS", "15");
        }
        let result = generate_android_project_with_backend(
            &temp_dir,
            "my-bench-project",
            "sample_fns::fibonacci",
            crate::FfiBackend::NativeCAbi,
        );
        unsafe {
            std::env::remove_var("MOBENCH_ANDROID_BENCHMARK_TIMEOUT_SECS");
            std::env::remove_var("MOBENCH_ANDROID_HEARTBEAT_INTERVAL_SECS");
        }
        result.expect("generate Android project");

        let android_test =
            fs::read_to_string(temp_dir.join(
                "android/app/src/androidTest/java/dev/world/mybenchproject/MainActivityTest.kt",
            ))
            .expect("read MainActivityTest.kt");

        assert!(android_test.contains("TimeUnit.SECONDS.toMillis(7200)"));
        assert!(android_test.contains("TimeUnit.SECONDS.toMillis(15)"));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_validate_no_unreplaced_placeholders() {
        // Should pass with no placeholders
        assert!(validate_no_unreplaced_placeholders("hello world", Path::new("test.txt")).is_ok());

        // Should pass with Gradle variables (not our placeholders)
        assert!(validate_no_unreplaced_placeholders("${ENV_VAR}", Path::new("test.txt")).is_ok());

        // Should fail with unreplaced template placeholders
        let result = validate_no_unreplaced_placeholders("hello {{NAME}}", Path::new("test.txt"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("{{NAME}}"));
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("my-project"), "MyProject");
        assert_eq!(to_pascal_case("my_project"), "MyProject");
        assert_eq!(to_pascal_case("myproject"), "Myproject");
        assert_eq!(to_pascal_case("my-bench-project"), "MyBenchProject");
    }

    #[test]
    fn test_detect_default_function_finds_benchmark() {
        let temp_dir = env::temp_dir().join("mobench-sdk-detect-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(temp_dir.join("src")).unwrap();

        // Create a lib.rs with a benchmark function
        let lib_content = r#"
use mobench_sdk::benchmark;

/// Some docs
#[benchmark]
fn my_benchmark_func() {
    // benchmark code
}

fn helper_func() {}
"#;
        fs::write(temp_dir.join("src/lib.rs"), lib_content).unwrap();
        fs::write(temp_dir.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let result = detect_default_function(&temp_dir, "my_crate");
        assert_eq!(result, Some("my_crate::my_benchmark_func".to_string()));

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_detect_default_function_no_benchmark() {
        let temp_dir = env::temp_dir().join("mobench-sdk-detect-none-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(temp_dir.join("src")).unwrap();

        // Create a lib.rs without benchmark functions
        let lib_content = r#"
fn regular_function() {
    // no benchmark here
}
"#;
        fs::write(temp_dir.join("src/lib.rs"), lib_content).unwrap();

        let result = detect_default_function(&temp_dir, "my_crate");
        assert!(result.is_none());

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_detect_default_function_pub_fn() {
        let temp_dir = env::temp_dir().join("mobench-sdk-detect-pub-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(temp_dir.join("src")).unwrap();

        // Create a lib.rs with a public benchmark function
        let lib_content = r#"
#[benchmark]
pub fn public_bench() {
    // benchmark code
}
"#;
        fs::write(temp_dir.join("src/lib.rs"), lib_content).unwrap();

        let result = detect_default_function(&temp_dir, "test-crate");
        assert_eq!(result, Some("test_crate::public_bench".to_string()));

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_resolve_default_function_fallback() {
        let temp_dir = env::temp_dir().join("mobench-sdk-resolve-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // No lib.rs exists, should fall back to default
        let result = resolve_default_function(&temp_dir, "my-crate", None);
        assert_eq!(result, "my_crate::example_benchmark");

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_sanitize_bundle_id_component() {
        // Hyphens should be removed
        assert_eq!(sanitize_bundle_id_component("bench-mobile"), "benchmobile");
        // Underscores should be removed
        assert_eq!(sanitize_bundle_id_component("bench_mobile"), "benchmobile");
        // Mixed separators should all be removed
        assert_eq!(
            sanitize_bundle_id_component("my-project_name"),
            "myprojectname"
        );
        // Already valid should remain unchanged (but lowercase)
        assert_eq!(sanitize_bundle_id_component("benchmobile"), "benchmobile");
        // Numbers should be preserved
        assert_eq!(sanitize_bundle_id_component("bench2mobile"), "bench2mobile");
        // Uppercase should be lowercased
        assert_eq!(sanitize_bundle_id_component("BenchMobile"), "benchmobile");
        // Complex case
        assert_eq!(
            sanitize_bundle_id_component("My-Complex_Project-123"),
            "mycomplexproject123"
        );
    }

    #[test]
    fn test_generate_ios_project_bundle_id_not_duplicated() {
        let temp_dir = env::temp_dir().join("mobench-sdk-ios-bundle-test");
        // Clean up any previous test run
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Use a crate name that would previously cause duplication
        let crate_name = "bench-mobile";
        let bundle_prefix = "dev.world.benchmobile";
        let project_pascal = "BenchRunner";

        let result = generate_ios_project(
            &temp_dir,
            crate_name,
            project_pascal,
            bundle_prefix,
            "bench_mobile::test_func",
        );
        assert!(
            result.is_ok(),
            "generate_ios_project failed: {:?}",
            result.err()
        );

        // Verify project.yml was created
        let project_yml_path = temp_dir.join("ios/BenchRunner/project.yml");
        assert!(project_yml_path.exists(), "project.yml should exist");

        // Read and verify the bundle ID is correct (not duplicated)
        let project_yml = fs::read_to_string(&project_yml_path).unwrap();

        // The bundle ID should be "dev.world.benchmobile.BenchRunner"
        // NOT "dev.world.benchmobile.benchmobile"
        assert!(
            project_yml.contains("dev.world.benchmobile.BenchRunner"),
            "Bundle ID should be 'dev.world.benchmobile.BenchRunner', got:\n{}",
            project_yml
        );
        assert!(
            !project_yml.contains("dev.world.benchmobile.benchmobile"),
            "Bundle ID should NOT be duplicated as 'dev.world.benchmobile.benchmobile', got:\n{}",
            project_yml
        );
        assert!(
            project_yml.contains("embed: false"),
            "Static xcframework dependency should be link-only, got:\n{}",
            project_yml
        );

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_generate_ios_project_preserves_existing_resources_on_regeneration() {
        let temp_dir = env::temp_dir().join("mobench-sdk-ios-resources-regenerate-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        generate_ios_project(
            &temp_dir,
            "bench_mobile",
            "BenchRunner",
            "dev.world.benchmobile",
            "bench_mobile::bench_prepare",
        )
        .unwrap();

        let resources_dir = temp_dir.join("ios/BenchRunner/BenchRunner/Resources");
        fs::create_dir_all(resources_dir.join("nested")).unwrap();
        fs::write(
            resources_dir.join("bench_spec.json"),
            r#"{"function":"bench_mobile::bench_prove","iterations":2,"warmup":1}"#,
        )
        .unwrap();
        fs::write(
            resources_dir.join("bench_meta.json"),
            r#"{"build_id":"build-123"}"#,
        )
        .unwrap();
        fs::write(resources_dir.join("nested/custom.txt"), "keep me").unwrap();

        generate_ios_project(
            &temp_dir,
            "bench_mobile",
            "BenchRunner",
            "dev.world.benchmobile",
            "bench_mobile::bench_prepare",
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(resources_dir.join("bench_spec.json")).unwrap(),
            r#"{"function":"bench_mobile::bench_prove","iterations":2,"warmup":1}"#
        );
        assert_eq!(
            fs::read_to_string(resources_dir.join("bench_meta.json")).unwrap(),
            r#"{"build_id":"build-123"}"#
        );
        assert_eq!(
            fs::read_to_string(resources_dir.join("nested/custom.txt")).unwrap(),
            "keep me"
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_generate_ios_native_backend_runner_template() {
        let temp_dir = env::temp_dir().join("mobench-sdk-ios-native-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        generate_ios_project_with_backend(
            &temp_dir,
            "native_benchmark",
            "BenchRunner",
            "dev.world.nativebenchmark",
            "native_benchmark::bench_prove",
            crate::FfiBackend::NativeCAbi,
        )
        .unwrap();

        let ffi =
            fs::read_to_string(temp_dir.join("ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift"))
                .unwrap();
        assert!(ffi.contains("mobench_run_benchmark_json"));
        assert!(ffi.contains("mobench_free_buf"));
        assert!(ffi.contains("BENCH_FUNCTION"));
        assert!(
            !ffi.contains("runBenchmark(spec:"),
            "native iOS runner must call the JSON C ABI, not UniFFI runBenchmark"
        );
        assert!(
            !ffi.contains("let report: BenchReport"),
            "native iOS runner should operate on JSON, not UniFFI BenchReport"
        );

        let header = fs::read_to_string(
            temp_dir.join("ios/BenchRunner/BenchRunner/Generated/native_benchmarkFFI.h"),
        )
        .unwrap();
        assert!(header.contains("mobench_run_benchmark_json"));
        let bridging_header = fs::read_to_string(
            temp_dir.join("ios/BenchRunner/BenchRunner/BenchRunner-Bridging-Header.h"),
        )
        .unwrap();
        assert!(bridging_header.contains("#import \"native_benchmarkFFI.h\""));
        let package_manifest =
            fs::read_to_string(temp_dir.join("ios/BenchRunner/project.yml")).unwrap();
        assert!(package_manifest.contains("../native_benchmark.xcframework"));
        assert!(package_manifest.contains("SWIFT_OBJC_BRIDGING_HEADER"));
        assert!(!package_manifest.contains("uniffi"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_generate_ios_boltffi_backend_runner_template() {
        let temp_dir = env::temp_dir().join("mobench-sdk-ios-boltffi-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        generate_ios_project_with_backend(
            &temp_dir,
            "bolt_benchmark",
            "BenchRunner",
            "dev.world.boltbenchmark",
            "bolt_benchmark::bench_prove",
            crate::FfiBackend::BoltFfi,
        )
        .unwrap();

        let ffi =
            fs::read_to_string(temp_dir.join("ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift"))
                .unwrap();
        assert!(ffi.contains("runBenchmarkJson(specJson:"));
        assert!(ffi.contains("BENCH_FUNCTION"));
        assert!(
            !ffi.contains("runBenchmark(spec:"),
            "BoltFFI iOS runner must call BoltFFI JSON bindings, not UniFFI runBenchmark"
        );
        assert!(
            !ffi.contains("mobench_run_benchmark_json"),
            "BoltFFI iOS runner must not call the native C ABI bridge"
        );
        let bridging_header = fs::read_to_string(
            temp_dir.join("ios/BenchRunner/BenchRunner/BenchRunner-Bridging-Header.h"),
        )
        .unwrap();
        assert!(
            !bridging_header.contains("#import"),
            "BoltFFI iOS runner should import the generated C module from Swift, not a UniFFI-style bridging header"
        );

        let boltffi_toml = fs::read_to_string(temp_dir.join("boltffi.toml")).unwrap();
        assert!(boltffi_toml.contains("module_name = \"BenchRunner\""));
        assert!(boltffi_toml.contains("ffi_module_name = \"bolt_benchmarkFFI\""));
        assert!(boltffi_toml.contains(
            "output = \"target/mobench/ios/BenchRunner/BenchRunner/Generated/BoltFFIGenerated\""
        ));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_ensure_ios_project_refreshes_existing_content_view_template() {
        let temp_dir = env::temp_dir().join("mobench-sdk-ios-refresh-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        ensure_ios_project_with_options(&temp_dir, "sample-fns", None, None)
            .expect("initial iOS project generation should succeed");

        let content_view_path = temp_dir.join("ios/BenchRunner/BenchRunner/ContentView.swift");
        assert!(content_view_path.exists(), "ContentView.swift should exist");

        fs::write(&content_view_path, "stale generated content").unwrap();

        ensure_ios_project_with_options(&temp_dir, "sample-fns", None, None)
            .expect("refreshing existing iOS project should succeed");

        let refreshed = fs::read_to_string(&content_view_path).unwrap();
        assert!(
            refreshed.contains("ProfileLaunchOptions"),
            "refreshed ContentView.swift should contain the latest profiling template, got:\n{}",
            refreshed
        );
        assert!(
            refreshed.contains("repeatUntilMs"),
            "refreshed ContentView.swift should contain repeat-until profiling support, got:\n{}",
            refreshed
        );
        assert!(
            refreshed.contains("Task.detached(priority: .userInitiated)"),
            "refreshed ContentView.swift should run benchmarks off the main actor, got:\n{}",
            refreshed
        );
        assert!(
            refreshed.contains("await MainActor.run"),
            "refreshed ContentView.swift should apply UI updates on the main actor, got:\n{}",
            refreshed
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_ensure_ios_project_refreshes_existing_ui_test_timeout_template() {
        let temp_dir = env::temp_dir().join("mobench-sdk-ios-uitest-refresh-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        ensure_ios_project_with_options(&temp_dir, "sample-fns", None, None)
            .expect("initial iOS project generation should succeed");

        let ui_test_path =
            temp_dir.join("ios/BenchRunner/BenchRunnerUITests/BenchRunnerUITests.swift");
        assert!(
            ui_test_path.exists(),
            "BenchRunnerUITests.swift should exist"
        );

        fs::write(&ui_test_path, "stale generated content").unwrap();

        ensure_ios_project_with_options(&temp_dir, "sample-fns", None, None)
            .expect("refreshing existing iOS project should succeed");

        let refreshed = fs::read_to_string(&ui_test_path).unwrap();
        assert!(
            refreshed.contains("private let defaultBenchmarkTimeout: TimeInterval = 300.0"),
            "refreshed BenchRunnerUITests.swift should include the default timeout, got:\n{}",
            refreshed
        );
        assert!(
            refreshed.contains(
                "ProcessInfo.processInfo.environment[\"MOBENCH_IOS_BENCHMARK_TIMEOUT_SECS\"]"
            ),
            "refreshed BenchRunnerUITests.swift should honor runtime timeout overrides, got:\n{}",
            refreshed
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_generate_ios_project_uses_configured_benchmark_timeout() {
        let temp_dir = env::temp_dir().join("mobench-sdk-ios-timeout-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let result = generate_ios_project_with_timeout(
            &temp_dir,
            "sample_fns",
            "BenchRunner",
            "dev.world.samplefns",
            "sample_fns::example_benchmark",
            1200,
            crate::FfiBackend::Uniffi,
        );

        assert!(result.is_ok(), "generate_ios_project should succeed");

        let ui_test_path =
            temp_dir.join("ios/BenchRunner/BenchRunnerUITests/BenchRunnerUITests.swift");
        let contents = fs::read_to_string(&ui_test_path).unwrap();
        assert!(
            contents.contains("private let defaultBenchmarkTimeout: TimeInterval = 1200.0"),
            "generated BenchRunnerUITests.swift should embed the configured timeout, got:\n{}",
            contents
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_resolve_ios_benchmark_timeout_secs_defaults_invalid_values() {
        assert_eq!(resolve_ios_benchmark_timeout_secs(None), 300);
        assert_eq!(resolve_ios_benchmark_timeout_secs(Some("900")), 900);
        assert_eq!(resolve_ios_benchmark_timeout_secs(Some("0")), 300);
        assert_eq!(resolve_ios_benchmark_timeout_secs(Some("bogus")), 300);
    }

    #[test]
    fn test_cross_platform_naming_consistency() {
        // Test that Android and iOS use the same naming convention for package/bundle IDs
        let temp_dir = env::temp_dir().join("mobench-sdk-naming-consistency-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let project_name = "bench-mobile";

        // Generate Android project
        let result = generate_android_project(&temp_dir, project_name, "bench_mobile::test_func");
        assert!(
            result.is_ok(),
            "generate_android_project failed: {:?}",
            result.err()
        );

        // Generate iOS project (mimicking how ensure_ios_project does it)
        let bundle_id_component = sanitize_bundle_id_component(project_name);
        let bundle_prefix = format!("dev.world.{}", bundle_id_component);
        let result = generate_ios_project(
            &temp_dir,
            &project_name.replace('-', "_"),
            "BenchRunner",
            &bundle_prefix,
            "bench_mobile::test_func",
        );
        assert!(
            result.is_ok(),
            "generate_ios_project failed: {:?}",
            result.err()
        );

        // Read Android build.gradle to extract package name
        let android_build_gradle = fs::read_to_string(temp_dir.join("android/app/build.gradle"))
            .expect("Failed to read Android build.gradle");

        // Read iOS project.yml to extract bundle ID prefix
        let ios_project_yml = fs::read_to_string(temp_dir.join("ios/BenchRunner/project.yml"))
            .expect("Failed to read iOS project.yml");

        // Both should use "benchmobile" (without hyphens or underscores)
        // Android: namespace = "dev.world.benchmobile"
        // iOS: bundleIdPrefix: dev.world.benchmobile
        assert!(
            android_build_gradle.contains("dev.world.benchmobile"),
            "Android package should be 'dev.world.benchmobile', got:\n{}",
            android_build_gradle
        );
        assert!(
            ios_project_yml.contains("dev.world.benchmobile"),
            "iOS bundle prefix should contain 'dev.world.benchmobile', got:\n{}",
            ios_project_yml
        );

        // Ensure Android doesn't use hyphens or underscores in the package ID component
        assert!(
            !android_build_gradle.contains("dev.world.bench-mobile"),
            "Android package should NOT contain hyphens"
        );
        assert!(
            !android_build_gradle.contains("dev.world.bench_mobile"),
            "Android package should NOT contain underscores"
        );

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_cross_platform_version_consistency() {
        // Test that Android and iOS use the same version strings
        let temp_dir = env::temp_dir().join("mobench-sdk-version-consistency-test");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let project_name = "test-project";

        // Generate Android project
        let result = generate_android_project(&temp_dir, project_name, "test_project::test_func");
        assert!(
            result.is_ok(),
            "generate_android_project failed: {:?}",
            result.err()
        );

        // Generate iOS project
        let bundle_id_component = sanitize_bundle_id_component(project_name);
        let bundle_prefix = format!("dev.world.{}", bundle_id_component);
        let result = generate_ios_project(
            &temp_dir,
            &project_name.replace('-', "_"),
            "BenchRunner",
            &bundle_prefix,
            "test_project::test_func",
        );
        assert!(
            result.is_ok(),
            "generate_ios_project failed: {:?}",
            result.err()
        );

        // Read Android build.gradle
        let android_build_gradle = fs::read_to_string(temp_dir.join("android/app/build.gradle"))
            .expect("Failed to read Android build.gradle");

        // Read iOS project.yml
        let ios_project_yml = fs::read_to_string(temp_dir.join("ios/BenchRunner/project.yml"))
            .expect("Failed to read iOS project.yml");

        // Both should use version "1.0.0"
        assert!(
            android_build_gradle.contains("versionName \"1.0.0\""),
            "Android versionName should be '1.0.0', got:\n{}",
            android_build_gradle
        );
        assert!(
            ios_project_yml.contains("CFBundleShortVersionString: \"1.0.0\""),
            "iOS CFBundleShortVersionString should be '1.0.0', got:\n{}",
            ios_project_yml
        );

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_bundle_id_prefix_consistency() {
        // Test that the bundle ID prefix format is consistent across platforms
        let test_cases = vec![
            ("my-project", "dev.world.myproject"),
            ("bench_mobile", "dev.world.benchmobile"),
            ("TestApp", "dev.world.testapp"),
            ("app-with-many-dashes", "dev.world.appwithmanydashes"),
            (
                "app_with_many_underscores",
                "dev.world.appwithmanyunderscores",
            ),
        ];

        for (input, expected_prefix) in test_cases {
            let sanitized = sanitize_bundle_id_component(input);
            let full_prefix = format!("dev.world.{}", sanitized);
            assert_eq!(
                full_prefix, expected_prefix,
                "For input '{}', expected '{}' but got '{}'",
                input, expected_prefix, full_prefix
            );
        }
    }
}
