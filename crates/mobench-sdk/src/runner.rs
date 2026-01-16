//! Benchmark execution runtime
//!
//! This module provides the execution engine that runs registered benchmarks
//! and collects timing data.

use crate::registry::find_benchmark;
use crate::types::{BenchError, BenchSpec, RunnerReport};
use mobench_runner::run_closure;

/// Runs a benchmark by name
///
/// Looks up the benchmark function in the registry and executes it with the
/// given specification.
///
/// # Arguments
///
/// * `spec` - Benchmark specification including function name, iterations, and warmup
///
/// # Returns
///
/// * `Ok(BenchReport)` - Report containing timing samples
/// * `Err(BenchError)` - If the function is not found or execution fails
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::{BenchSpec, run_benchmark};
///
/// let spec = BenchSpec {
///     name: "my_benchmark".to_string(),
///     iterations: 100,
///     warmup: 10,
/// };
///
/// let report = run_benchmark(spec)?;
/// println!("Mean: {} ns", report.mean());
/// ```
pub fn run_benchmark(spec: BenchSpec) -> Result<RunnerReport, BenchError> {
    // Find the benchmark function in the registry
    let bench_fn =
        find_benchmark(&spec.name).ok_or_else(|| BenchError::UnknownFunction(spec.name.clone()))?;

    // Create a closure that invokes the registered function
    let closure =
        || (bench_fn.invoke)(&[]).map_err(|e| mobench_runner::BenchError::Execution(e.to_string()));

    // Run the benchmark using mobench-runner's timing infrastructure
    let report = run_closure(spec, closure)?;

    Ok(report)
}

/// Builder for constructing and running benchmarks
///
/// Provides a fluent interface for configuring benchmark parameters.
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::BenchmarkBuilder;
///
/// let report = BenchmarkBuilder::new("my_benchmark")
///     .iterations(100)
///     .warmup(10)
///     .run()?;
/// ```
#[derive(Debug, Clone)]
pub struct BenchmarkBuilder {
    function: String,
    iterations: u32,
    warmup: u32,
}

impl BenchmarkBuilder {
    /// Creates a new benchmark builder
    ///
    /// # Arguments
    ///
    /// * `function` - Name of the benchmark function to run
    pub fn new(function: impl Into<String>) -> Self {
        Self {
            function: function.into(),
            iterations: 100, // Default
            warmup: 10,      // Default
        }
    }

    /// Sets the number of iterations
    ///
    /// # Arguments
    ///
    /// * `n` - Number of times to run the benchmark (after warmup)
    pub fn iterations(mut self, n: u32) -> Self {
        self.iterations = n;
        self
    }

    /// Sets the number of warmup iterations
    ///
    /// # Arguments
    ///
    /// * `n` - Number of warmup runs (not measured)
    pub fn warmup(mut self, n: u32) -> Self {
        self.warmup = n;
        self
    }

    /// Runs the benchmark and returns the report
    ///
    /// # Returns
    ///
    /// * `Ok(BenchReport)` - Report containing timing samples
    /// * `Err(BenchError)` - If the function is not found or execution fails
    pub fn run(self) -> Result<RunnerReport, BenchError> {
        let spec = BenchSpec {
            name: self.function,
            iterations: self.iterations,
            warmup: self.warmup,
        };

        run_benchmark(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_defaults() {
        let builder = BenchmarkBuilder::new("test_fn");
        assert_eq!(builder.iterations, 100);
        assert_eq!(builder.warmup, 10);
    }

    #[test]
    fn test_builder_customization() {
        let builder = BenchmarkBuilder::new("test_fn").iterations(50).warmup(5);
        assert_eq!(builder.iterations, 50);
        assert_eq!(builder.warmup, 5);
    }
}
