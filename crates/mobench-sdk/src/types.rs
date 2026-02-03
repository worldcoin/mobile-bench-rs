//! Core types for mobench-sdk.
//!
//! This module defines the fundamental types used throughout the SDK:
//!
//! - [`BenchError`] - Error types for benchmark and build operations
//! - [`Target`] - Platform selection (Android, iOS, or both)
//! - [`BuildConfig`] / [`BuildProfile`] - Build configuration options
//! - [`BuildResult`] - Output from build operations
//! - [`InitConfig`] - Project initialization settings
//!
//! ## Re-exports from timing module
//!
//! For convenience, this module also re-exports types from [`crate::timing`]:
//!
//! - [`BenchSpec`] - Benchmark specification (name, iterations, warmup)
//! - [`BenchSample`] - Single timing measurement
//! - [`RunnerReport`] - Complete benchmark results

// Re-export timing types for convenience
pub use crate::timing::{
    BenchReport as RunnerReport, BenchSample, BenchSpec, BenchSummary, TimingError as RunnerError,
};

use std::path::PathBuf;

/// Error types for mobench-sdk operations.
///
/// This enum covers all error conditions that can occur during
/// benchmark registration, execution, and mobile app building.
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::{run_benchmark, BenchSpec, BenchError};
///
/// let spec = BenchSpec {
///     name: "nonexistent".to_string(),
///     iterations: 10,
///     warmup: 1,
/// };
///
/// match run_benchmark(spec) {
///     Ok(report) => println!("Success!"),
///     Err(BenchError::UnknownFunction(name)) => {
///         eprintln!("Benchmark '{}' not found", name);
///     }
///     Err(e) => eprintln!("Other error: {}", e),
/// }
/// ```
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    /// Error from the underlying benchmark runner.
    ///
    /// This wraps errors from [`crate::timing::TimingError`], such as
    /// zero iterations or execution failures.
    #[error("benchmark runner error: {0}")]
    Runner(#[from] crate::timing::TimingError),

    /// The requested benchmark function was not found in the registry.
    ///
    /// This occurs when calling [`run_benchmark`](crate::run_benchmark) with
    /// a function name that hasn't been registered via `#[benchmark]`.
    ///
    /// The error includes a list of available benchmarks to help diagnose the issue.
    #[error("unknown benchmark function: '{0}'. Available benchmarks: {1:?}\n\nEnsure the function is:\n  1. Annotated with #[benchmark]\n  2. Public (pub fn)\n  3. Takes no parameters and returns ()")]
    UnknownFunction(String, Vec<String>),

    /// An error occurred during benchmark execution.
    ///
    /// This is a catch-all for execution-time errors that don't fit
    /// other categories.
    #[error("benchmark execution failed: {0}")]
    Execution(String),

    /// An I/O error occurred.
    ///
    /// Common causes include missing files, permission issues, or
    /// disk space problems during build operations.
    #[error("I/O error: {0}. Check file paths and permissions")]
    Io(#[from] std::io::Error),

    /// JSON serialization or deserialization failed.
    ///
    /// This can occur when reading/writing benchmark specifications
    /// or configuration files.
    #[error("serialization error: {0}. Check JSON validity or output serializability")]
    Serialization(#[from] serde_json::Error),

    /// A configuration error occurred.
    ///
    /// This indicates invalid or missing configuration, such as
    /// malformed TOML files or missing required fields.
    #[error("configuration error: {0}. Check mobench.toml or CLI flags")]
    Config(String),

    /// A build error occurred.
    ///
    /// This covers failures during mobile app building, including:
    /// - Missing build tools (cargo-ndk, xcodebuild, etc.)
    /// - Compilation errors
    /// - Code signing failures
    /// - Missing dependencies
    #[error("build error: {0}")]
    Build(String),
}

/// Target platform for benchmarks.
///
/// Specifies which mobile platform(s) to build for or run benchmarks on.
///
/// # Example
///
/// ```
/// use mobench_sdk::Target;
///
/// let target = Target::Android;
/// assert_eq!(target.as_str(), "android");
///
/// let both = Target::Both;
/// assert_eq!(both.as_str(), "both");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Android platform (APK with native .so libraries).
    Android,
    /// iOS platform (xcframework with static libraries).
    Ios,
    /// Both Android and iOS platforms.
    Both,
}

impl Target {
    /// Returns the string representation of the target.
    ///
    /// # Returns
    ///
    /// - `"android"` for [`Target::Android`]
    /// - `"ios"` for [`Target::Ios`]
    /// - `"both"` for [`Target::Both`]
    pub fn as_str(&self) -> &'static str {
        match self {
            Target::Android => "android",
            Target::Ios => "ios",
            Target::Both => "both",
        }
    }
}

/// Configuration for initializing a new benchmark project.
///
/// Used by the `cargo mobench init` command to generate project scaffolding.
///
/// # Example
///
/// ```
/// use mobench_sdk::{InitConfig, Target};
/// use std::path::PathBuf;
///
/// let config = InitConfig {
///     target: Target::Android,
///     project_name: "my-benchmarks".to_string(),
///     output_dir: PathBuf::from("./bench-mobile"),
///     generate_examples: true,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct InitConfig {
    /// Target platform(s) to initialize for.
    pub target: Target,
    /// Name of the benchmark project/crate.
    pub project_name: String,
    /// Output directory for generated files.
    pub output_dir: PathBuf,
    /// Whether to generate example benchmark functions.
    pub generate_examples: bool,
}

/// Configuration for building mobile apps.
///
/// Controls the build process including target platform, optimization level,
/// and caching behavior.
///
/// # Example
///
/// ```
/// use mobench_sdk::{BuildConfig, BuildProfile, Target};
///
/// // Release build for Android
/// let config = BuildConfig {
///     target: Target::Android,
///     profile: BuildProfile::Release,
///     incremental: true,
/// };
///
/// // Debug build for iOS
/// let ios_config = BuildConfig {
///     target: Target::Ios,
///     profile: BuildProfile::Debug,
///     incremental: false,  // Force rebuild
/// };
/// ```
#[derive(Debug, Clone)]
pub struct BuildConfig {
    /// Target platform to build for.
    pub target: Target,
    /// Build profile (debug or release).
    pub profile: BuildProfile,
    /// If `true`, skip rebuilding if artifacts already exist.
    pub incremental: bool,
}

/// Build profile controlling optimization and debug info.
///
/// Similar to Cargo's `--release` flag, this controls whether the build
/// is optimized for debugging or performance.
///
/// # Example
///
/// ```
/// use mobench_sdk::BuildProfile;
///
/// let debug = BuildProfile::Debug;
/// assert_eq!(debug.as_str(), "debug");
///
/// let release = BuildProfile::Release;
/// assert_eq!(release.as_str(), "release");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    /// Debug build with debug symbols and no optimizations.
    ///
    /// Faster compilation but slower runtime. Useful for development
    /// and troubleshooting.
    Debug,
    /// Release build with optimizations enabled.
    ///
    /// Slower compilation but faster runtime. Use this for actual
    /// benchmark measurements.
    Release,
}

impl BuildProfile {
    /// Returns the string representation of the profile.
    ///
    /// # Returns
    ///
    /// - `"debug"` for [`BuildProfile::Debug`]
    /// - `"release"` for [`BuildProfile::Release`]
    pub fn as_str(&self) -> &'static str {
        match self {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        }
    }
}

/// Result of a successful build operation.
///
/// Contains paths to the built artifacts, which can be used for
/// deployment to BrowserStack or local testing.
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::builders::AndroidBuilder;
///
/// let result = builder.build(&config)?;
///
/// println!("App built at: {:?}", result.app_path);
/// if let Some(test_suite) = result.test_suite_path {
///     println!("Test suite at: {:?}", test_suite);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct BuildResult {
    /// Platform that was built.
    pub platform: Target,
    /// Path to the main app artifact.
    ///
    /// - Android: Path to the APK file
    /// - iOS: Path to the xcframework directory
    pub app_path: PathBuf,
    /// Path to the test suite artifact, if applicable.
    ///
    /// - Android: Path to the androidTest APK (for Espresso)
    /// - iOS: Path to the XCUITest runner zip
    pub test_suite_path: Option<PathBuf>,
}
