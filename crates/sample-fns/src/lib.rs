//! Sample benchmark functions for mobile testing using UniFFI (proc macro mode).

use mobench_sdk::timing::{run_closure, TimingError};

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

// Conversion from mobench-sdk timing types
impl From<mobench_sdk::timing::BenchSpec> for BenchSpec {
    fn from(spec: mobench_sdk::timing::BenchSpec) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

impl From<BenchSpec> for mobench_sdk::timing::BenchSpec {
    fn from(spec: BenchSpec) -> Self {
        Self {
            name: spec.name,
            iterations: spec.iterations,
            warmup: spec.warmup,
        }
    }
}

impl From<mobench_sdk::timing::BenchSample> for BenchSample {
    fn from(sample: mobench_sdk::timing::BenchSample) -> Self {
        Self {
            duration_ns: sample.duration_ns,
        }
    }
}

impl From<mobench_sdk::timing::BenchReport> for BenchReport {
    fn from(report: mobench_sdk::timing::BenchReport) -> Self {
        Self {
            spec: report.spec.into(),
            samples: report.samples.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<TimingError> for BenchError {
    fn from(err: TimingError) -> Self {
        match err {
            TimingError::NoIterations { .. } => BenchError::InvalidIterations,
            TimingError::Execution(msg) => BenchError::ExecutionFailed { reason: msg },
        }
    }
}

/// Run a benchmark by name with the given specification.
#[uniffi::export]
pub fn run_benchmark(spec: BenchSpec) -> Result<BenchReport, BenchError> {
    let timing_spec: mobench_sdk::timing::BenchSpec = spec.into();

    let report = match timing_spec.name.as_str() {
        "fibonacci" | "fib" | "sample_fns::fibonacci" => {
            run_closure(timing_spec, || {
                let result = fibonacci_batch(30, 1000);
                // Use the result to prevent optimization
                std::hint::black_box(result);
                Ok(())
            })
            .map_err(|e: TimingError| -> BenchError { e.into() })?
        }
        "checksum" | "checksum_1k" | "sample_fns::checksum" => {
            run_closure(timing_spec, || {
                // Run checksum 10000 times to make it measurable
                let mut sum = 0u64;
                for _ in 0..10000 {
                    sum = sum.wrapping_add(checksum(&CHECKSUM_INPUT));
                }
                // Use the result to prevent optimization
                std::hint::black_box(sum);
                Ok(())
            })
            .map_err(|e: TimingError| -> BenchError { e.into() })?
        }
        _ => {
            return Err(BenchError::UnknownFunction {
                name: timing_spec.name.clone(),
            })
        }
    };

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
    fn test_run_benchmark_fibonacci() {
        let spec = BenchSpec {
            name: "fibonacci".to_string(),
            iterations: 3,
            warmup: 1,
        };
        let report = run_benchmark(spec).unwrap();
        assert_eq!(report.samples.len(), 3);
        assert_eq!(report.spec.name, "fibonacci");
    }

    #[test]
    fn test_run_benchmark_checksum() {
        let spec = BenchSpec {
            name: "checksum".to_string(),
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
            name: "fibonacci".to_string(),
            iterations: 0,
            warmup: 0,
        };
        let result = run_benchmark(spec);
        assert!(matches!(result, Err(BenchError::InvalidIterations)));
    }
}
