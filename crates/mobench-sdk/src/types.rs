//! Core types for bench-sdk
//!
//! This module re-exports types from mobench-runner and adds SDK-specific types.

// Re-export mobench-runner types for convenience
pub use mobench_runner::{
    BenchError as RunnerError, BenchReport as RunnerReport, BenchSample, BenchSpec,
};

use std::path::PathBuf;

/// Error types for bench-sdk operations
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    /// Error from the benchmark runner
    #[error("runner error: {0}")]
    Runner(#[from] mobench_runner::BenchError),

    /// Benchmark function not found in registry
    #[error("unknown benchmark function: {0}")]
    UnknownFunction(String),

    /// Error during benchmark execution
    #[error("execution error: {0}")]
    Execution(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Configuration error
    #[error("configuration error: {0}")]
    Config(String),

    /// Build error
    #[error("build error: {0}")]
    Build(String),
}

/// Target platform for benchmarks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Android platform
    Android,
    /// iOS platform
    Ios,
    /// Both platforms
    Both,
}

impl Target {
    pub fn as_str(&self) -> &'static str {
        match self {
            Target::Android => "android",
            Target::Ios => "ios",
            Target::Both => "both",
        }
    }
}

/// Configuration for initializing a benchmark project
#[derive(Debug, Clone)]
pub struct InitConfig {
    /// Target platform(s)
    pub target: Target,
    /// Project name
    pub project_name: String,
    /// Output directory for generated files
    pub output_dir: PathBuf,
    /// Whether to generate example benchmarks
    pub generate_examples: bool,
}

/// Configuration for building mobile apps
#[derive(Debug, Clone)]
pub struct BuildConfig {
    /// Target platform
    pub target: Target,
    /// Build profile (debug or release)
    pub profile: BuildProfile,
    /// Whether to skip build if artifacts exist
    pub incremental: bool,
}

/// Build profile
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    /// Debug build
    Debug,
    /// Release build
    Release,
}

impl BuildProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        }
    }
}

/// Result of a build operation
#[derive(Debug, Clone)]
pub struct BuildResult {
    /// Platform that was built
    pub platform: Target,
    /// Path to the app artifact (APK, IPA, etc.)
    pub app_path: PathBuf,
    /// Path to the test suite artifact (if applicable)
    pub test_suite_path: Option<PathBuf>,
}
