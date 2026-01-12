//! Code generation and template management
//!
//! This module provides functionality for generating mobile app projects from
//! embedded templates. It handles template parameterization and file generation.

use crate::types::{BenchError, InitConfig, Target};
use std::fs;
use std::path::{Path, PathBuf};

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

    // Create base directories
    fs::create_dir_all(output_dir)?;

    // Generate bench-mobile FFI wrapper crate
    generate_bench_mobile_crate(output_dir, &config.project_name)?;

    // Generate platform-specific projects
    match config.target {
        Target::Android => {
            generate_android_project(output_dir, &config.project_name)?;
        }
        Target::Ios => {
            generate_ios_project(output_dir, &config.project_name)?;
        }
        Target::Both => {
            generate_android_project(output_dir, &config.project_name)?;
            generate_ios_project(output_dir, &config.project_name)?;
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
    fs::create_dir_all(&crate_dir.join("src"))?;

    // Generate Cargo.toml
    let cargo_toml = format!(
        r#"[package]
name = "{}-bench-mobile"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[dependencies]
bench-sdk = "0.1"
uniffi = "0.28"

# Add your project's crate here
# {} = {{ path = ".." }}

[build-dependencies]
uniffi = {{ version = "0.28", features = ["build"] }}
"#,
        project_name, project_name
    );

    fs::write(crate_dir.join("Cargo.toml"), cargo_toml)?;

    // Generate src/lib.rs
    let lib_rs = r#"//! Mobile FFI bindings for benchmarks
//!
//! This crate provides the FFI boundary between Rust benchmarks and mobile
//! platforms (Android/iOS). It uses UniFFI to generate type-safe bindings.

use uniffi;

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
fn generate_android_project(_output_dir: &Path, _project_name: &str) -> Result<(), BenchError> {
    // TODO: Implement Android project generation
    // This will extract templates from the embedded templates/ directory
    // For now, return Ok as a placeholder
    println!("Android project generation not yet implemented");
    Ok(())
}

/// Generates iOS project structure
fn generate_ios_project(_output_dir: &Path, _project_name: &str) -> Result<(), BenchError> {
    // TODO: Implement iOS project generation
    // This will extract templates from the embedded templates/ directory
    // For now, return Ok as a placeholder
    println!("iOS project generation not yet implemented");
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
