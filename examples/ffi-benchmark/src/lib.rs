//! FFI benchmark example demonstrating UniFFI integration.
//!
//! This example shows how to define a full FFI surface (types, errors, and
//! `run_benchmark`) for Kotlin/Swift bindings. For the minimal SDK-only usage,
//! see `examples/basic-benchmark`.

// Alternative: The mobench_sdk::ffi module provides pre-defined types that match
// what UniFFI expects. You can use these directly or as templates for your own types:
//
//   use mobench_sdk::ffi::{BenchSpecFfi, BenchReportFfi, BenchErrorFfi, run_benchmark_ffi};
//
// This example defines its own types for demonstration, but the ffi module
// is a simpler starting point for most use cases.

use mobench_sdk::benchmark;

const CHECKSUM_INPUT: [u8; 1024] = [1; 1024];

/// Specification for a benchmark run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct BenchSpec {
    pub name: String,
    pub iterations: u32,
    pub warmup: u32,
}

/// A single benchmark sample with timing information.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct BenchSample {
    pub duration_ns: u64,
}

/// Complete benchmark report with spec and timing samples.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct BenchReport {
    pub spec: BenchSpec,
    pub samples: Vec<BenchSample>,
}

/// Error types for benchmark operations.
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

// Generate UniFFI scaffolding from proc macros
uniffi::setup_scaffolding!();

// Conversion from mobench-sdk types
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
            mobench_sdk::BenchError::Runner(runner_err) => BenchError::ExecutionFailed {
                reason: runner_err.to_string(),
            },
            mobench_sdk::BenchError::UnknownFunction(name, _available) => {
                BenchError::UnknownFunction { name }
            }
            _ => BenchError::ExecutionFailed {
                reason: err.to_string(),
            },
        }
    }
}

/// Run a benchmark by name with the given specification.
///
/// This is the main FFI entry point called from mobile platforms.
#[uniffi::export]
pub fn run_benchmark(spec: BenchSpec) -> Result<BenchReport, BenchError> {
    let sdk_spec: mobench_sdk::BenchSpec = spec.into();
    let report = mobench_sdk::run_benchmark(sdk_spec)?;
    Ok(report.into())
}

/// Compute fibonacci number iteratively.
pub fn fibonacci(n: u32) -> u64 {
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

/// Compute fibonacci in a more measurable way by doing it multiple times.
pub fn fibonacci_batch(n: u32, iterations: u32) -> u64 {
    let mut result = 0u64;
    for _ in 0..iterations {
        result = result.wrapping_add(fibonacci(n));
    }
    result
}

/// Compute checksum by summing all bytes.
pub fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().map(|&b| b as u64).sum()
}

// ============================================================================
// Benchmark Functions
// ============================================================================
// These functions are marked with #[benchmark] and automatically registered
// with mobench-sdk's registry system.

/// Benchmark: Fibonacci calculation (30th number, 1000 iterations)
#[benchmark]
pub fn bench_fibonacci() {
    let result = fibonacci_batch(30, 1000);
    std::hint::black_box(result);
}

/// Benchmark: Checksum calculation on 1KB data (10000 iterations)
#[benchmark]
pub fn bench_checksum() {
    let mut sum = 0u64;
    for _ in 0..10000 {
        sum = sum.wrapping_add(checksum(&CHECKSUM_INPUT));
    }
    std::hint::black_box(sum);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fib_sequence() {
        assert_eq!(fibonacci(0), 0);
        assert_eq!(fibonacci(1), 1);
        assert_eq!(fibonacci(10), 55);
        assert_eq!(fibonacci(24), 46368);
    }

    #[test]
    fn checksum_matches() {
        assert_eq!(checksum(&CHECKSUM_INPUT), 1024);
    }

    #[test]
    fn test_run_benchmark_via_registry() {
        // Test that benchmarks can be discovered via the registry
        let benchmarks = mobench_sdk::discover_benchmarks();
        assert!(benchmarks.len() >= 2, "Should find at least 2 benchmarks");

        // Test execution via FFI using registry name
        let spec = BenchSpec {
            name: "ffi_benchmark::bench_fibonacci".to_string(),
            iterations: 3,
            warmup: 1,
        };
        let report = run_benchmark(spec).unwrap();
        assert_eq!(report.samples.len(), 3);
    }

    #[test]
    fn test_run_benchmark_checksum() {
        let spec = BenchSpec {
            name: "ffi_benchmark::bench_checksum".to_string(),
            iterations: 2,
            warmup: 0,
        };
        let report = run_benchmark(spec).unwrap();
        assert_eq!(report.samples.len(), 2);
    }

    #[test]
    fn test_unknown_function_error() {
        let spec = BenchSpec {
            name: "unknown".to_string(),
            iterations: 1,
            warmup: 0,
        };
        let result = run_benchmark(spec);
        assert!(matches!(result, Err(BenchError::UnknownFunction { .. })));
    }

    #[test]
    fn test_invalid_iterations() {
        let spec = BenchSpec {
            name: "ffi_benchmark::bench_fibonacci".to_string(),
            iterations: 0,
            warmup: 0,
        };
        let result = run_benchmark(spec);
        assert!(matches!(result, Err(BenchError::ExecutionFailed { .. })));
    }
}
