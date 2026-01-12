//! Code generation and template management
//!
//! This module provides functionality for generating mobile app projects from
//! embedded templates. It handles template parameterization and file generation.

use crate::types::{BenchError, InitConfig, Target};
use std::fs;
use std::path::{Path, PathBuf};

use include_dir::{Dir, DirEntry, include_dir};

const ANDROID_TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../templates/android");
const IOS_TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../templates/ios");

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
    let bundle_prefix = format!("dev.world.{}", project_slug);

    // Create base directories
    fs::create_dir_all(output_dir)?;

    // Generate bench-mobile FFI wrapper crate
    generate_bench_mobile_crate(output_dir, &project_slug)?;

    // Generate platform-specific projects
    match config.target {
        Target::Android => {
            generate_android_project(output_dir, &project_slug)?;
        }
        Target::Ios => {
            generate_ios_project(output_dir, &project_slug, &project_pascal, &bundle_prefix)?;
        }
        Target::Both => {
            generate_android_project(output_dir, &project_slug)?;
            generate_ios_project(output_dir, &project_slug, &project_pascal, &bundle_prefix)?;
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
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[dependencies]
bench-sdk = {{ path = ".." }}
uniffi = "0.28"
{} = {{ path = ".." }}

[features]
default = []

[build-dependencies]
uniffi = {{ version = "0.28", features = ["build"] }}
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

// Re-export bench-sdk types with UniFFI annotations
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

// Convert from bench-sdk types
impl From<bench_sdk::BenchSpec> for BenchSpec {
    fn from(spec: bench_sdk::BenchSpec) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

impl From<BenchSpec> for bench_sdk::BenchSpec {
    fn from(spec: BenchSpec) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

impl From<bench_sdk::BenchSample> for BenchSample {
    fn from(sample: bench_sdk::BenchSample) -> Self {
        Self {
            duration_ns: sample.duration_ns,
        }
    }
}

impl From<bench_sdk::RunnerReport> for BenchReport {
    fn from(report: bench_sdk::RunnerReport) -> Self {
        Self {
            spec: report.spec.into(),
            samples: report.samples.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<bench_sdk::BenchError> for BenchError {
    fn from(err: bench_sdk::BenchError) -> Self {
        match err {
            bench_sdk::BenchError::Runner(runner_err) => {
                BenchError::ExecutionFailed {
                    reason: runner_err.to_string(),
                }
            }
            bench_sdk::BenchError::UnknownFunction(name) => {
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
    let sdk_spec: bench_sdk::BenchSpec = spec.into();
    let report = bench_sdk::run_benchmark(sdk_spec)?;
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

    Ok(())
}

/// Generates Android project structure
fn generate_android_project(output_dir: &Path, project_slug: &str) -> Result<(), BenchError> {
    let target_dir = output_dir.join("android");
    let vars = vec![
        TemplateVar {
            name: "PACKAGE_NAME",
            value: format!("dev.world.{}", project_slug),
        },
        TemplateVar {
            name: "UNIFFI_NAMESPACE",
            value: project_slug.replace('-', "_"),
        },
        TemplateVar {
            name: "LIBRARY_NAME",
            value: project_slug.replace('-', "_"),
        },
        TemplateVar {
            name: "DEFAULT_FUNCTION",
            value: "example_fibonacci".to_string(),
        },
    ];
    render_dir(&ANDROID_TEMPLATES, Path::new(""), &target_dir, &vars)?;
    Ok(())
}

/// Generates iOS project structure
fn generate_ios_project(
    output_dir: &Path,
    project_slug: &str,
    project_pascal: &str,
    bundle_prefix: &str,
) -> Result<(), BenchError> {
    let target_dir = output_dir.join("ios");
    let vars = vec![
        TemplateVar {
            name: "DEFAULT_FUNCTION",
            value: "example_fibonacci".to_string(),
        },
        TemplateVar {
            name: "PROJECT_NAME_PASCAL",
            value: project_pascal.to_string(),
        },
        TemplateVar {
            name: "BUNDLE_ID_PREFIX",
            value: bundle_prefix.to_string(),
        },
        TemplateVar {
            name: "BUNDLE_ID",
            value: format!("{}.{}", bundle_prefix, project_slug),
        },
        TemplateVar {
            name: "LIBRARY_NAME",
            value: project_slug.replace('-', "_"),
        },
    ];
    render_dir(&IOS_TEMPLATES, Path::new(""), &target_dir, &vars)?;
    Ok(())
}

/// Generates bench-sdk.toml configuration file
fn generate_config_file(output_dir: &Path, config: &InitConfig) -> Result<(), BenchError> {
    let config_content = format!(
        r#"# Bench SDK Configuration
# This file controls how benchmarks are built and executed

[project]
name = "{}"
target = "{}"

[build]
profile = "debug"  # or "release"

# BrowserStack configuration (optional)
# Uncomment and fill in your credentials to use BrowserStack
# [browserstack]
# username = "${{BROWSERSTACK_USERNAME}}"
# access_key = "${{BROWSERSTACK_ACCESS_KEY}}"
# project = "{}-benchmarks"

# Device matrix (optional)
# Uncomment to specify devices for BrowserStack runs
# [[devices]]
# name = "Pixel 7"
# os = "android"
# os_version = "13.0"
# tags = ["default"]

# [[devices]]
# name = "iPhone 14"
# os = "ios"
# os_version = "16"
# tags = ["default"]
"#,
        config.project_name,
        config.target.as_str(),
        config.project_name
    );

    fs::write(output_dir.join("bench-sdk.toml"), config_content)?;

    Ok(())
}

/// Generates example benchmark functions
fn generate_example_benchmarks(output_dir: &Path) -> Result<(), BenchError> {
    let examples_dir = output_dir.join("benches");
    fs::create_dir_all(&examples_dir)?;

    let example_content = r#"//! Example benchmarks
//!
//! This file demonstrates how to write benchmarks with bench-sdk.

use bench_sdk::benchmark;

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

fn render_dir(
    dir: &Dir,
    prefix: &Path,
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
                let next_prefix = prefix.join(sub.path());
                render_dir(sub, &next_prefix, out_root, vars)?;
            }
            DirEntry::File(file) => {
                if file.path().components().any(|c| c.as_os_str() == ".gradle") {
                    continue;
                }
                let mut relative = prefix.join(file.path());
                let mut contents = file.contents().to_vec();
                if let Some(ext) = relative.extension()
                    && ext == "template"
                {
                    relative.set_extension("");
                    let rendered = render_template(
                        std::str::from_utf8(&contents).map_err(|e| {
                            BenchError::Build(format!(
                                "invalid UTF-8 in template {:?}: {}",
                                file.path(),
                                e
                            ))
                        })?,
                        vars,
                    );
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

fn render_template(input: &str, vars: &[TemplateVar]) -> String {
    let mut output = input.to_string();
    for var in vars {
        output = output.replace(&format!("{{{{{}}}}}", var.name), &var.value);
    }
    output
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

fn to_pascal_case(input: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_generate_bench_mobile_crate() {
        let temp_dir = env::temp_dir().join("bench-sdk-test");
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
}
