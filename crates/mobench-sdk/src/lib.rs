//! # mobench-sdk
//!
//! A mobile benchmarking SDK for Rust that enables running performance benchmarks
//! on real Android and iOS devices via BrowserStack App Automate.
//!
//! ## Overview
//!
//! `mobench-sdk` provides a simple, declarative API for defining benchmarks that can
//! run on mobile devices. It handles the complexity of cross-compilation, FFI bindings,
//! and mobile app packaging automatically.
//!
//! ## Quick Start
//!
//! ### 1. Add Dependencies
//!
//! ```toml
//! [dependencies]
//! mobench-sdk = "0.1"
//! inventory = "0.3"  # Required for benchmark registration
//! ```
//!
//! ### 2. Define Benchmarks
//!
//! Use the [`#[benchmark]`](macro@benchmark) attribute to mark functions for benchmarking:
//!
//! ```ignore
//! use mobench_sdk::benchmark;
//!
//! #[benchmark]
//! fn my_expensive_operation() {
//!     let result = expensive_computation();
//!     std::hint::black_box(result);  // Prevent optimization
//! }
//!
//! #[benchmark]
//! fn another_benchmark() {
//!     for i in 0..1000 {
//!         std::hint::black_box(i * i);
//!     }
//! }
//! ```
//!
//! ### 3. Build and Run
//!
//! Use the `mobench` CLI to build and run benchmarks:
//!
//! ```bash
//! # Install the CLI
//! cargo install mobench
//!
//! # Build for Android (outputs to target/mobench/)
//! cargo mobench build --target android
//!
//! # Build for iOS
//! cargo mobench build --target ios
//!
//! # Run on BrowserStack
//! cargo mobench run --target android --function my_expensive_operation \
//!     --iterations 100 --warmup 10 --devices "Google Pixel 7-13.0"
//! ```
//!
//! ## Architecture
//!
//! The SDK consists of several components:
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`timing`] | Core timing infrastructure (always available) |
//! | [`registry`] | Runtime discovery of `#[benchmark]` functions (requires `full` feature) |
//! | [`runner`] | Benchmark execution engine (requires `full` feature) |
//! | [`builders`] | Android and iOS build automation (requires `full` feature) |
//! | [`codegen`] | Mobile app template generation (requires `full` feature) |
//! | [`types`] | Common types and error definitions |
//!
//! ## Crate Ecosystem
//!
//! The mobench ecosystem consists of three crates:
//!
//! - **`mobench-sdk`** (this crate) - Core SDK library with timing harness and build automation
//! - **[`mobench`](https://crates.io/crates/mobench)** - CLI tool for building and running benchmarks
//! - **[`mobench-macros`](https://crates.io/crates/mobench-macros)** - `#[benchmark]` proc macro
//!
//! ## Feature Flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `full` | Yes | Full SDK with build automation, templates, and registry |
//! | `runner-only` | No | Minimal timing-only mode for mobile binaries |
//!
//! For mobile binaries where binary size matters, use `runner-only`:
//!
//! ```toml
//! [dependencies]
//! mobench-sdk = { version = "0.1", default-features = false, features = ["runner-only"] }
//! ```
//!
//! ## Programmatic Usage
//!
//! You can also use the SDK programmatically:
//!
//! ### Using the Builder Pattern
//!
//! ```ignore
//! use mobench_sdk::BenchmarkBuilder;
//!
//! let report = BenchmarkBuilder::new("my_benchmark")
//!     .iterations(100)
//!     .warmup(10)
//!     .run()?;
//!
//! println!("Mean: {} ns", report.samples.iter()
//!     .map(|s| s.duration_ns)
//!     .sum::<u64>() / report.samples.len() as u64);
//! ```
//!
//! ### Using BenchSpec Directly
//!
//! ```ignore
//! use mobench_sdk::{BenchSpec, run_benchmark};
//!
//! let spec = BenchSpec {
//!     name: "my_benchmark".to_string(),
//!     iterations: 50,
//!     warmup: 5,
//! };
//!
//! let report = run_benchmark(spec)?;
//! println!("Collected {} samples", report.samples.len());
//! ```
//!
//! ### Discovering Benchmarks
//!
//! ```ignore
//! use mobench_sdk::{discover_benchmarks, list_benchmark_names};
//!
//! // Get all registered benchmark names
//! let names = list_benchmark_names();
//! for name in names {
//!     println!("Found benchmark: {}", name);
//! }
//!
//! // Get full benchmark function info
//! let benchmarks = discover_benchmarks();
//! for bench in benchmarks {
//!     println!("Benchmark: {}", bench.name);
//! }
//! ```
//!
//! ## Building Mobile Apps
//!
//! The SDK includes builders for automating mobile app creation:
//!
//! ### Android Builder
//!
//! ```ignore
//! use mobench_sdk::builders::AndroidBuilder;
//! use mobench_sdk::{BuildConfig, BuildProfile, Target};
//!
//! let builder = AndroidBuilder::new(".", "my-bench-crate")
//!     .verbose(true)
//!     .output_dir("target/mobench");  // Default
//!
//! let config = BuildConfig {
//!     target: Target::Android,
//!     profile: BuildProfile::Release,
//!     incremental: true,
//! };
//!
//! let result = builder.build(&config)?;
//! println!("APK built at: {:?}", result.app_path);
//! ```
//!
//! ### iOS Builder
//!
//! ```ignore
//! use mobench_sdk::builders::{IosBuilder, SigningMethod};
//! use mobench_sdk::{BuildConfig, BuildProfile, Target};
//!
//! let builder = IosBuilder::new(".", "my-bench-crate")
//!     .verbose(true);
//!
//! let config = BuildConfig {
//!     target: Target::Ios,
//!     profile: BuildProfile::Release,
//!     incremental: true,
//! };
//!
//! let result = builder.build(&config)?;
//! println!("xcframework built at: {:?}", result.app_path);
//!
//! // Package IPA for distribution
//! let ipa_path = builder.package_ipa("BenchRunner", SigningMethod::AdHoc)?;
//! ```
//!
//! ## Output Directory
//!
//! By default, all mobile artifacts are written to `target/mobench/`:
//!
//! ```text
//! target/mobench/
//! ├── android/
//! │   ├── app/
//! │   │   ├── src/main/jniLibs/     # Native .so libraries
//! │   │   └── build/outputs/apk/    # Built APK
//! │   └── ...
//! └── ios/
//!     ├── sample_fns.xcframework/   # Built xcframework
//!     ├── BenchRunner/              # Xcode project
//!     └── BenchRunner.ipa           # Packaged IPA
//! ```
//!
//! This keeps generated files inside `target/`, following Rust conventions
//! and preventing accidental commits of mobile project files.
//!
//! ## Platform Requirements
//!
//! ### Android
//!
//! - Android NDK (set `ANDROID_NDK_HOME` environment variable)
//! - `cargo-ndk` (`cargo install cargo-ndk`)
//! - Rust targets: `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`
//!
//! ### iOS
//!
//! - Xcode with command line tools
//! - `uniffi-bindgen` (`cargo install uniffi-bindgen`)
//! - `xcodegen` (optional, `brew install xcodegen`)
//! - Rust targets: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
//!
//! ## Best Practices
//!
//! ### Use `black_box` to Prevent Optimization
//!
//! Always wrap benchmark results with [`std::hint::black_box`] to prevent the
//! compiler from optimizing away the computation:
//!
//! ```ignore
//! #[benchmark]
//! fn correct_benchmark() {
//!     let result = expensive_computation();
//!     std::hint::black_box(result);  // Result is "used"
//! }
//! ```
//!
//! ### Avoid Side Effects
//!
//! Benchmarks should be deterministic and avoid I/O operations:
//!
//! ```ignore
//! // Good: Pure computation
//! #[benchmark]
//! fn good_benchmark() {
//!     let data = vec![1, 2, 3, 4, 5];
//!     let sum: i32 = data.iter().sum();
//!     std::hint::black_box(sum);
//! }
//!
//! // Avoid: File I/O adds noise
//! #[benchmark]
//! fn noisy_benchmark() {
//!     let data = std::fs::read_to_string("data.txt").unwrap();  // Don't do this
//!     std::hint::black_box(data);
//! }
//! ```
//!
//! ### Choose Appropriate Iteration Counts
//!
//! - **Warmup**: 5-10 iterations to warm CPU caches and JIT
//! - **Iterations**: 50-100 for stable statistics
//! - Mobile devices may have more variance than desktop
//!
//! ## License
//!
//! MIT License - see repository for details.

// Core timing module - always available
pub mod timing;
pub mod types;

// Full SDK modules - only with "full" feature
#[cfg(feature = "full")]
pub mod builders;
#[cfg(feature = "full")]
pub mod codegen;
#[cfg(feature = "full")]
pub mod registry;
#[cfg(feature = "full")]
pub mod runner;

// Re-export the benchmark macro from bench-macros (only with full feature)
#[cfg(feature = "full")]
pub use mobench_macros::benchmark;

// Re-export key types for convenience (full feature)
#[cfg(feature = "full")]
pub use registry::{BenchFunction, discover_benchmarks, find_benchmark, list_benchmark_names};
#[cfg(feature = "full")]
pub use runner::{BenchmarkBuilder, run_benchmark};

// Re-export types that are always available
pub use types::{BenchError, BenchSample, BenchSpec, RunnerReport};

// Re-export types that require full feature
#[cfg(feature = "full")]
pub use types::{BuildConfig, BuildProfile, BuildResult, InitConfig, Target};

// Re-export timing types at the crate root for convenience
pub use timing::{run_closure, TimingError};

/// Library version, matching `Cargo.toml`.
///
/// This can be used to verify SDK compatibility:
///
/// ```
/// assert!(!mobench_sdk::VERSION.is_empty());
/// ```
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_is_set() {
        assert!(!VERSION.is_empty());
    }

    #[cfg(feature = "full")]
    #[test]
    fn test_discover_benchmarks_compiles() {
        // This test just ensures the function is accessible
        let _benchmarks = discover_benchmarks();
    }
}
