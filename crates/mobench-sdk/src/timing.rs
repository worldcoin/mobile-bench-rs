//! Lightweight benchmarking harness for mobile platforms.
//!
//! This module provides the core timing infrastructure for the mobench ecosystem.
//! It was previously a separate crate (`mobench-runner`) but has been consolidated
//! into `mobench-sdk` for a simpler dependency graph.
//!
//! The module is designed to be minimal and portable, with no platform-specific
//! dependencies, making it suitable for compilation to Android and iOS targets.
//!
//! ## Overview
//!
//! The timing module executes benchmark functions with:
//! - Configurable warmup iterations
//! - Precise nanosecond-resolution timing
//! - Simple, serializable results
//!
//! ## Usage
//!
//! Most users should use this via the higher-level [`crate::run_benchmark`] function
//! or [`crate::BenchmarkBuilder`]. Direct usage is for custom integrations:
//!
//! ```
//! use mobench_sdk::timing::{BenchSpec, run_closure, TimingError};
//!
//! // Define a benchmark specification
//! let spec = BenchSpec::new("my_benchmark", 100, 10)?;
//!
//! // Run the benchmark
//! let report = run_closure(spec, || {
//!     // Your benchmark code
//!     let sum: u64 = (0..1000).sum();
//!     std::hint::black_box(sum);
//!     Ok(())
//! })?;
//!
//! // Analyze results
//! let mean_ns = report.samples.iter()
//!     .map(|s| s.duration_ns)
//!     .sum::<u64>() / report.samples.len() as u64;
//!
//! println!("Mean: {} ns", mean_ns);
//! # Ok::<(), TimingError>(())
//! ```
//!
//! ## Types
//!
//! | Type | Description |
//! |------|-------------|
//! | [`BenchSpec`] | Benchmark configuration (name, iterations, warmup) |
//! | [`BenchSample`] | Single timing measurement in nanoseconds |
//! | [`BenchReport`] | Complete results with all samples |
//! | [`TimingError`] | Error conditions during benchmarking |
//!
//! ## Feature Flags
//!
//! This module is always available. When using `mobench-sdk` with default features,
//! you also get build automation and template generation. For minimal binary size
//! (e.g., on mobile targets), use the `runner-only` feature:
//!
//! ```toml
//! [dependencies]
//! mobench-sdk = { version = "0.1", default-features = false, features = ["runner-only"] }
//! ```

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Benchmark specification defining what and how to benchmark.
///
/// Contains the benchmark name, number of measurement iterations, and
/// warmup iterations to perform before measuring.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::BenchSpec;
///
/// // Create a spec for 100 iterations with 10 warmup runs
/// let spec = BenchSpec::new("sorting_benchmark", 100, 10)?;
///
/// assert_eq!(spec.name, "sorting_benchmark");
/// assert_eq!(spec.iterations, 100);
/// assert_eq!(spec.warmup, 10);
/// # Ok::<(), mobench_sdk::timing::TimingError>(())
/// ```
///
/// # Serialization
///
/// `BenchSpec` implements `Serialize` and `Deserialize` for JSON persistence:
///
/// ```
/// use mobench_sdk::timing::BenchSpec;
///
/// let spec = BenchSpec {
///     name: "my_bench".to_string(),
///     iterations: 50,
///     warmup: 5,
/// };
///
/// let json = serde_json::to_string(&spec)?;
/// let restored: BenchSpec = serde_json::from_str(&json)?;
///
/// assert_eq!(spec.name, restored.name);
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchSpec {
    /// Name of the benchmark, typically the fully-qualified function name.
    ///
    /// Examples: `"my_crate::fibonacci"`, `"sorting_benchmark"`
    pub name: String,

    /// Number of iterations to measure.
    ///
    /// Each iteration produces one [`BenchSample`]. Must be greater than zero.
    pub iterations: u32,

    /// Number of warmup iterations before measurement.
    ///
    /// Warmup iterations are not recorded. They allow CPU caches to warm
    /// and any JIT compilation to complete. Can be zero.
    pub warmup: u32,
}

impl BenchSpec {
    /// Creates a new benchmark specification.
    ///
    /// # Arguments
    ///
    /// * `name` - Name identifier for the benchmark
    /// * `iterations` - Number of measured iterations (must be > 0)
    /// * `warmup` - Number of warmup iterations (can be 0)
    ///
    /// # Errors
    ///
    /// Returns [`TimingError::NoIterations`] if `iterations` is zero.
    ///
    /// # Example
    ///
    /// ```
    /// use mobench_sdk::timing::BenchSpec;
    ///
    /// let spec = BenchSpec::new("test", 100, 10)?;
    /// assert_eq!(spec.iterations, 100);
    ///
    /// // Zero iterations is an error
    /// let err = BenchSpec::new("test", 0, 10);
    /// assert!(err.is_err());
    /// # Ok::<(), mobench_sdk::timing::TimingError>(())
    /// ```
    pub fn new(name: impl Into<String>, iterations: u32, warmup: u32) -> Result<Self, TimingError> {
        if iterations == 0 {
            return Err(TimingError::NoIterations { count: iterations });
        }

        Ok(Self {
            name: name.into(),
            iterations,
            warmup,
        })
    }
}

/// A single timing sample from a benchmark iteration.
///
/// Contains the elapsed time in nanoseconds for one execution of the
/// benchmark function.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::BenchSample;
///
/// let sample = BenchSample { duration_ns: 1_500_000 };
///
/// // Convert to milliseconds
/// let ms = sample.duration_ns as f64 / 1_000_000.0;
/// assert_eq!(ms, 1.5);
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchSample {
    /// Duration of the iteration in nanoseconds.
    ///
    /// Measured using [`std::time::Instant`] for monotonic, high-resolution timing.
    pub duration_ns: u64,
}

impl BenchSample {
    /// Creates a sample from a [`Duration`].
    fn from_duration(duration: Duration) -> Self {
        Self {
            duration_ns: duration.as_nanos() as u64,
        }
    }
}

/// Complete benchmark report with all timing samples.
///
/// Contains the original specification and all collected samples.
/// Can be serialized to JSON for storage or transmission.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::{BenchSpec, run_closure};
///
/// let spec = BenchSpec::new("example", 50, 5)?;
/// let report = run_closure(spec, || {
///     std::hint::black_box(42);
///     Ok(())
/// })?;
///
/// // Calculate statistics
/// let samples: Vec<u64> = report.samples.iter()
///     .map(|s| s.duration_ns)
///     .collect();
///
/// let min = samples.iter().min().unwrap();
/// let max = samples.iter().max().unwrap();
/// let mean = samples.iter().sum::<u64>() / samples.len() as u64;
///
/// println!("Min: {} ns, Max: {} ns, Mean: {} ns", min, max, mean);
/// # Ok::<(), mobench_sdk::timing::TimingError>(())
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchReport {
    /// The specification used for this benchmark run.
    pub spec: BenchSpec,

    /// All collected timing samples.
    ///
    /// The length equals `spec.iterations`. Samples are in execution order.
    pub samples: Vec<BenchSample>,
}

/// Errors that can occur during benchmark execution.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::{BenchSpec, TimingError};
///
/// // Zero iterations produces an error
/// let result = BenchSpec::new("test", 0, 10);
/// assert!(matches!(result, Err(TimingError::NoIterations { .. })));
/// ```
#[derive(Debug, Error)]
pub enum TimingError {
    /// The iteration count was zero or invalid.
    ///
    /// At least one iteration is required to produce a measurement.
    /// The error includes the actual value provided for diagnostic purposes.
    #[error("iterations must be greater than zero (got {count}). Minimum recommended: 10")]
    NoIterations {
        /// The invalid iteration count that was provided.
        count: u32,
    },

    /// The benchmark function failed during execution.
    ///
    /// Contains a description of the failure.
    #[error("benchmark function failed: {0}")]
    Execution(String),
}

/// Runs a benchmark by executing a closure repeatedly.
///
/// This is the core benchmarking function. It:
///
/// 1. Executes the closure `spec.warmup` times without recording
/// 2. Executes the closure `spec.iterations` times, recording each duration
/// 3. Returns a [`BenchReport`] with all samples
///
/// # Arguments
///
/// * `spec` - Benchmark configuration specifying iterations and warmup
/// * `f` - Closure to benchmark; must return `Result<(), TimingError>`
///
/// # Returns
///
/// A [`BenchReport`] containing all timing samples, or a [`TimingError`] if
/// the benchmark fails.
///
/// # Example
///
/// ```
/// use mobench_sdk::timing::{BenchSpec, run_closure, TimingError};
///
/// let spec = BenchSpec::new("sum_benchmark", 100, 10)?;
///
/// let report = run_closure(spec, || {
///     let sum: u64 = (0..1000).sum();
///     std::hint::black_box(sum);
///     Ok(())
/// })?;
///
/// assert_eq!(report.samples.len(), 100);
///
/// // Calculate mean duration
/// let total_ns: u64 = report.samples.iter().map(|s| s.duration_ns).sum();
/// let mean_ns = total_ns / report.samples.len() as u64;
/// println!("Mean: {} ns", mean_ns);
/// # Ok::<(), TimingError>(())
/// ```
///
/// # Error Handling
///
/// If the closure returns an error, the benchmark stops immediately:
///
/// ```
/// use mobench_sdk::timing::{BenchSpec, run_closure, TimingError};
///
/// let spec = BenchSpec::new("failing_bench", 100, 0)?;
///
/// let result = run_closure(spec, || {
///     Err(TimingError::Execution("simulated failure".into()))
/// });
///
/// assert!(result.is_err());
/// # Ok::<(), TimingError>(())
/// ```
///
/// # Timing Precision
///
/// Uses [`std::time::Instant`] for timing, which provides monotonic,
/// nanosecond-resolution measurements on most platforms.
pub fn run_closure<F>(spec: BenchSpec, mut f: F) -> Result<BenchReport, TimingError>
where
    F: FnMut() -> Result<(), TimingError>,
{
    if spec.iterations == 0 {
        return Err(TimingError::NoIterations {
            count: spec.iterations,
        });
    }

    // Warmup phase - not measured
    for _ in 0..spec.warmup {
        f()?;
    }

    // Measurement phase
    let mut samples = Vec::with_capacity(spec.iterations as usize);
    for _ in 0..spec.iterations {
        let start = Instant::now();
        f()?;
        samples.push(BenchSample::from_duration(start.elapsed()));
    }

    Ok(BenchReport { spec, samples })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_benchmark() {
        let spec = BenchSpec::new("noop", 3, 1).unwrap();
        let report = run_closure(spec, || Ok(())).unwrap();

        assert_eq!(report.samples.len(), 3);
        let non_zero = report.samples.iter().filter(|s| s.duration_ns > 0).count();
        assert!(non_zero >= 1);
    }

    #[test]
    fn rejects_zero_iterations() {
        let result = BenchSpec::new("test", 0, 10);
        assert!(matches!(result, Err(TimingError::NoIterations { count: 0 })));
    }

    #[test]
    fn allows_zero_warmup() {
        let spec = BenchSpec::new("test", 5, 0).unwrap();
        assert_eq!(spec.warmup, 0);

        let report = run_closure(spec, || Ok(())).unwrap();
        assert_eq!(report.samples.len(), 5);
    }

    #[test]
    fn serializes_to_json() {
        let spec = BenchSpec::new("test", 10, 2).unwrap();
        let report = run_closure(spec, || Ok(())).unwrap();

        let json = serde_json::to_string(&report).unwrap();
        let restored: BenchReport = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.spec.name, "test");
        assert_eq!(restored.samples.len(), 10);
    }
}
