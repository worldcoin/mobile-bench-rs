//! Mobile Benchmark SDK for Rust
//!
//! `bench-sdk` is a library for benchmarking Rust functions on real mobile devices
//! (Android and iOS) via BrowserStack. It provides a simple API similar to criterion.rs
//! but targets mobile platforms.
//!
//! # Quick Start
//!
//! 1. Add bench-sdk to your project:
//! ```toml
//! [dependencies]
//! bench-sdk = "0.1"
//! ```
//!
//! 2. Mark functions with `#[benchmark]`:
//! ```ignore
//! use mobench_sdk::benchmark;
//!
//! #[benchmark]
//! fn my_expensive_operation() {
//!     // Your code here
//!     let result = compute_something();
//!     std::hint::black_box(result);
//! }
//! ```
//!
//! 3. Initialize mobile project:
//! ```bash
//! cargo bench-sdk init --target android
//! ```
//!
//! 4. Build and run:
//! ```bash
//! cargo bench-sdk build --target android
//! cargo bench-sdk run my_expensive_operation --target android
//! ```
//!
//! # Architecture
//!
//! The SDK consists of several components:
//!
//! - **Registry**: Discovers functions marked with `#[benchmark]` at runtime
//! - **Runner**: Executes benchmarks and collects timing data
//! - **Builders**: Automates building Android/iOS apps
//! - **Codegen**: Generates mobile app templates
//!
//! # Example: Programmatic Usage
//!
//! ```ignore
//! use mobench_sdk::{BenchmarkBuilder, BenchSpec};
//!
//! fn main() -> Result<(), mobench_sdk::BenchError> {
//!     // Using the builder pattern
//!     let report = BenchmarkBuilder::new("my_benchmark")
//!         .iterations(100)
//!         .warmup(10)
//!         .run()?;
//!
//!     println!("Samples: {}", report.samples.len());
//!
//!     // Or using BenchSpec directly
//!     let spec = BenchSpec {
//!         name: "my_benchmark".to_string(),
//!         iterations: 50,
//!         warmup: 5,
//!     };
//!     let report = mobench_sdk::run_benchmark(spec)?;
//!
//!     Ok(())
//! }
//! ```

// Public modules
pub mod builders;
pub mod codegen;
pub mod registry;
pub mod runner;
pub mod types;

// Re-export the benchmark macro from bench-macros
pub use mobench_macros::benchmark;

// Re-export key types for convenience
pub use registry::{BenchFunction, discover_benchmarks, find_benchmark, list_benchmark_names};
pub use runner::{BenchmarkBuilder, run_benchmark};
pub use types::{
    BenchError, BenchSample, BenchSpec, BuildConfig, BuildProfile, BuildResult, InitConfig,
    RunnerReport, Target,
};

// Re-export mobench-runner types for backward compatibility
pub use mobench_runner;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_is_set() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn test_discover_benchmarks_compiles() {
        // This test just ensures the function is accessible
        let _benchmarks = discover_benchmarks();
    }
}
