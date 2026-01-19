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
            generate_ios_project(output_dir, &project_slug, &project_pascal, &bundle_prefix, default_function)?;
        }
        Target::Both => {
            generate_android_project(output_dir, &project_slug, default_function)?;
            generate_ios_project(output_dir, &project_slug, &project_pascal, &bundle_prefix, default_function)?;
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
mobench-sdk = {{ path = ".." }}
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
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct BenchReport {
    pub spec: BenchSpec,
    pub samples: Vec<BenchSample>,
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
        }
    }
}

impl From<mobench_sdk::RunnerReport> for BenchReport {
    fn from(report: mobench_sdk::RunnerReport) -> Self {
        Self {
            spec: report.spec.into(),
            samples: report.samples.into_iter().map(Into::into).collect(),
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
            mobench_sdk::BenchError::UnknownFunction(name) => {
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
    let target_dir = output_dir.join("android");
    let library_name = project_slug.replace('-', "_");
    let project_pascal = to_pascal_case(project_slug);
    let package_name = format!("dev.world.{}", project_slug);
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
    ];
    render_dir(&ANDROID_TEMPLATES, &target_dir, &vars)?;

    // Move Kotlin files to the correct package directory structure
    // The package "dev.world.{project_slug}" maps to directory "dev/world/{project_slug}/"
    move_kotlin_files_to_package_dir(&target_dir, &package_name)?;

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
fn move_kotlin_files_to_package_dir(android_dir: &Path, package_name: &str) -> Result<(), BenchError> {
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
    let target_dir = output_dir.join("ios");
    // Sanitize bundle ID components to ensure they only contain alphanumeric characters
    // iOS bundle identifiers should not contain hyphens or underscores
    let sanitized_bundle_prefix = {
        let parts: Vec<&str> = bundle_prefix.split('.').collect();
        parts.iter()
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
    ];
    render_dir(&IOS_TEMPLATES, &target_dir, &vars)?;
    Ok(())
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
    "gradle", "xml", "kt", "java", "swift", "yml", "yaml", "json", "toml", "md", "txt", "h", "m",
    "plist", "pbxproj", "xcscheme", "xcworkspacedata", "entitlements", "modulemap",
];

fn render_dir(
    dir: &Dir,
    out_root: &Path,
    vars: &[TemplateVar],
) -> Result<(), BenchError> {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(sub) => {
                // Skip cache directories
                if sub.path().components().any(|c| c.as_os_str() == ".gradle") {
                    continue;
                }
                render_dir(sub, out_root, vars)?;
            }
            DirEntry::File(file) => {
                if file.path().components().any(|c| c.as_os_str() == ".gradle") {
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

                if should_render {
                    if let Ok(text) = std::str::from_utf8(&contents) {
                        let rendered = render_template(text, vars);
                        // Validate that all template variables were replaced
                        validate_no_unreplaced_placeholders(&rendered, &relative)?;
                        contents = rendered.into_bytes();
                    }
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
        if let Some(ext) = stem_path.extension() {
            if let Some(ext_str) = ext.to_str() {
                return TEMPLATE_EXTENSIONS.contains(&ext_str);
            }
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
        if dir.join("Cargo.toml").exists() {
            if let Some(detected) = detect_default_function(dir, &crate_name_normalized) {
                return detected;
            }
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
    if android_project_exists(output_dir) {
        return Ok(());
    }

    println!("Android project not found, generating scaffolding...");
    let project_slug = crate_name.replace('-', "_");

    // Resolve the default function by auto-detecting from source
    let effective_root = project_root.unwrap_or_else(|| {
        output_dir.parent().unwrap_or(output_dir)
    });
    let default_function = resolve_default_function(effective_root, crate_name, crate_dir);

    generate_android_project(output_dir, &project_slug, &default_function)?;
    println!("  Generated Android project at {:?}", output_dir.join("android"));
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
    if ios_project_exists(output_dir) {
        return Ok(());
    }

    println!("iOS project not found, generating scaffolding...");
    // Use fixed "BenchRunner" for project/scheme name to match template directory structure
    let project_pascal = "BenchRunner";
    // Derive library name and bundle prefix from crate name
    let library_name = crate_name.replace('-', "_");
    // Use sanitized bundle ID component (alphanumeric only) to avoid iOS validation issues
    // e.g., "bench-mobile" or "bench_mobile" -> "benchmobile"
    let bundle_id_component = sanitize_bundle_id_component(crate_name);
    let bundle_prefix = format!("dev.world.{}", bundle_id_component);

    // Resolve the default function by auto-detecting from source
    let effective_root = project_root.unwrap_or_else(|| {
        output_dir.parent().unwrap_or(output_dir)
    });
    let default_function = resolve_default_function(effective_root, crate_name, crate_dir);

    generate_ios_project(output_dir, &library_name, project_pascal, &bundle_prefix, &default_function)?;
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

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_generate_android_project_no_unreplaced_placeholders() {
        let temp_dir = env::temp_dir().join("mobench-sdk-android-test");
        // Clean up any previous test run
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let result = generate_android_project(&temp_dir, "my-bench-project", "my_bench_project::test_func");
        assert!(result.is_ok(), "generate_android_project failed: {:?}", result.err());

        // Verify key files exist
        let android_dir = temp_dir.join("android");
        assert!(android_dir.join("settings.gradle").exists());
        assert!(android_dir.join("app/build.gradle").exists());
        assert!(android_dir.join("app/src/main/AndroidManifest.xml").exists());
        assert!(android_dir.join("app/src/main/res/values/strings.xml").exists());
        assert!(android_dir.join("app/src/main/res/values/themes.xml").exists());

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
            let contents = fs::read_to_string(&path).expect(&format!("Failed to read {}", file));

            // Check for unreplaced placeholders
            let has_placeholder = contents.contains("{{") && contents.contains("}}");
            assert!(
                !has_placeholder,
                "File {} contains unreplaced template placeholders: {}",
                file,
                contents
            );
        }

        // Verify specific substitutions were made
        let settings = fs::read_to_string(android_dir.join("settings.gradle")).unwrap();
        assert!(
            settings.contains("my-bench-project-android") || settings.contains("my_bench_project-android"),
            "settings.gradle should contain project name"
        );

        let build_gradle = fs::read_to_string(android_dir.join("app/build.gradle")).unwrap();
        assert!(
            build_gradle.contains("dev.world.my-bench-project") || build_gradle.contains("dev.world.my_bench_project"),
            "build.gradle should contain package name"
        );

        let manifest = fs::read_to_string(android_dir.join("app/src/main/AndroidManifest.xml")).unwrap();
        assert!(
            manifest.contains("Theme.MyBenchProject"),
            "AndroidManifest.xml should contain PascalCase theme name"
        );

        let strings = fs::read_to_string(android_dir.join("app/src/main/res/values/strings.xml")).unwrap();
        assert!(
            strings.contains("Benchmark"),
            "strings.xml should contain app name with Benchmark"
        );

        // Verify Kotlin files are in the correct package directory structure
        // For package "dev.world.my-bench-project", files should be in "dev/world/my-bench-project/"
        let main_activity_path = android_dir.join("app/src/main/java/dev/world/my-bench-project/MainActivity.kt");
        assert!(
            main_activity_path.exists(),
            "MainActivity.kt should be in package directory: {:?}",
            main_activity_path
        );

        let test_activity_path = android_dir.join("app/src/androidTest/java/dev/world/my-bench-project/MainActivityTest.kt");
        assert!(
            test_activity_path.exists(),
            "MainActivityTest.kt should be in package directory: {:?}",
            test_activity_path
        );

        // Verify the files are NOT in the root java directory
        assert!(
            !android_dir.join("app/src/main/java/MainActivity.kt").exists(),
            "MainActivity.kt should not be in root java directory"
        );
        assert!(
            !android_dir.join("app/src/androidTest/java/MainActivityTest.kt").exists(),
            "MainActivityTest.kt should not be in root java directory"
        );

        // Cleanup
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
        assert_eq!(sanitize_bundle_id_component("my-project_name"), "myprojectname");
        // Already valid should remain unchanged (but lowercase)
        assert_eq!(sanitize_bundle_id_component("benchmobile"), "benchmobile");
        // Numbers should be preserved
        assert_eq!(sanitize_bundle_id_component("bench2mobile"), "bench2mobile");
        // Uppercase should be lowercased
        assert_eq!(sanitize_bundle_id_component("BenchMobile"), "benchmobile");
        // Complex case
        assert_eq!(sanitize_bundle_id_component("My-Complex_Project-123"), "mycomplexproject123");
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
        assert!(result.is_ok(), "generate_ios_project failed: {:?}", result.err());

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

        // Cleanup
        fs::remove_dir_all(&temp_dir).ok();
    }
}
