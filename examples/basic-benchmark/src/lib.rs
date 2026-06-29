//! Basic benchmark examples demonstrating mobench-sdk usage.
//!
//! This example keeps things minimal: register functions with #[benchmark] and
//! let the SDK handle discovery and execution. It also exposes the minimal
//! UniFFI/mobile entrypoint needed by the example CI so the generated Android
//! and iOS apps can execute the benchmarks end to end.

use mobench_sdk::benchmark;

const CHECKSUM_INPUT_LEN: usize = 64 * 1024;
const CHECKSUM_WINDOW_LEN: usize = 8 * 1024;
const CHECKSUM_SWEEP_ITERATIONS: usize = 2_048;
const FIBONACCI_START: u32 = 28;
const FIBONACCI_SPAN: u32 = 6;
const FIBONACCI_SWEEP_ITERATIONS: u32 = 200_000;
static CHECKSUM_INPUT: [u8; CHECKSUM_INPUT_LEN] = build_checksum_input();

const fn build_checksum_input() -> [u8; CHECKSUM_INPUT_LEN] {
    let mut bytes = [0u8; CHECKSUM_INPUT_LEN];
    let mut i = 0;
    let mut state = 0x1234_5678u32;
    while i < CHECKSUM_INPUT_LEN {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        bytes[i] = (state >> 16) as u8;
        i += 1;
    }
    bytes
}

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

uniffi::setup_scaffolding!();
mobench_sdk::export_native_c_abi!();

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

/// Sweep across a small fibonacci range to avoid a trivially constant workload.
pub fn fibonacci_sweep(start: u32, span: u32, iterations: u32) -> u64 {
    let span = if span == 0 { 1 } else { span };
    let mut result = 0u64;
    for i in 0..iterations {
        result = result.wrapping_add(fibonacci(start + (i % span)));
    }
    result
}

/// Sweep overlapping windows across the input buffer to keep the checksum work realistic.
pub fn checksum_sweep(bytes: &[u8], window_len: usize, iterations: usize) -> u64 {
    assert!(window_len > 0, "window_len must be greater than zero");
    assert!(
        bytes.len() >= window_len,
        "window_len must fit within the input buffer"
    );

    let max_start = bytes.len() - window_len;
    let mut sum = 0u64;
    for i in 0..iterations {
        let start = if max_start == 0 {
            0
        } else {
            i % (max_start + 1)
        };
        sum = sum.wrapping_add(checksum(&bytes[start..start + window_len]));
    }
    sum
}

// ============================================================================
// Benchmark Functions
// ============================================================================
// These functions are marked with #[benchmark] and automatically registered
// with mobench-sdk's registry system.

/// Benchmark: Fibonacci sweep with enough work to make mobile samples meaningful.
#[benchmark]
pub fn bench_fibonacci() {
    let start = std::hint::black_box(FIBONACCI_START);
    let span = std::hint::black_box(FIBONACCI_SPAN);
    let iterations = std::hint::black_box(FIBONACCI_SWEEP_ITERATIONS);
    let result = fibonacci_sweep(start, span, iterations);
    std::hint::black_box(result);
}

/// Benchmark: Sliding-window checksum over a larger input buffer.
#[benchmark]
pub fn bench_checksum() {
    let bytes = std::hint::black_box(&CHECKSUM_INPUT);
    let window_len = std::hint::black_box(CHECKSUM_WINDOW_LEN);
    let iterations = std::hint::black_box(CHECKSUM_SWEEP_ITERATIONS);
    let result = checksum_sweep(bytes, window_len, iterations);
    std::hint::black_box(result);
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
        let expected = CHECKSUM_INPUT[..4].iter().map(|&b| b as u64).sum::<u64>();
        assert_eq!(checksum(&CHECKSUM_INPUT[..4]), expected);
    }

    #[test]
    fn checksum_input_has_entropy() {
        assert_ne!(CHECKSUM_INPUT[0], CHECKSUM_INPUT[1]);
        assert_ne!(CHECKSUM_INPUT[512], CHECKSUM_INPUT[513]);
    }

    #[test]
    fn fibonacci_sweep_walks_expected_range() {
        let expected = fibonacci(4) + fibonacci(5) + fibonacci(6) + fibonacci(4) + fibonacci(5);
        assert_eq!(fibonacci_sweep(4, 3, 5), expected);
    }

    #[test]
    fn checksum_sweep_wraps_windows() {
        let bytes = [1u8, 2, 3, 4];
        let expected = checksum(&bytes[0..2])
            + checksum(&bytes[1..3])
            + checksum(&bytes[2..4])
            + checksum(&bytes[0..2]);
        assert_eq!(checksum_sweep(&bytes, 2, 4), expected);
    }

    #[test]
    fn test_discover_benchmarks() {
        let benchmarks: Vec<&mobench_sdk::BenchFunction> = mobench_sdk::discover_benchmarks();
        assert!(benchmarks.len() >= 2, "Should find at least 2 benchmarks");
    }

    #[test]
    fn test_run_benchmark_via_sdk() {
        let spec = mobench_sdk::BenchSpec {
            name: "basic_benchmark::bench_fibonacci".to_string(),
            iterations: 3,
            warmup: 1,
        };
        let report = mobench_sdk::run_benchmark(spec).unwrap();
        assert_eq!(report.samples.len(), 3);
    }

    #[test]
    fn test_run_benchmark_via_mobile_ffi() {
        let spec = BenchSpec {
            name: "basic_benchmark::bench_checksum".to_string(),
            iterations: 2,
            warmup: 0,
        };
        let report = run_benchmark(spec).unwrap();
        assert_eq!(report.samples.len(), 2);
    }

    #[test]
    fn test_run_benchmark_via_native_c_abi() {
        let spec = br#"{"name":"basic_benchmark::bench_checksum","iterations":2,"warmup":0}"#;
        let mut out = mobench_sdk::MobenchBuf::default();

        let status = unsafe { mobench_run_benchmark_json(spec.as_ptr(), spec.len(), &mut out) };

        assert_eq!(status, 0);
        assert!(!out.ptr.is_null());
        assert!(out.len > 0);

        let report_bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
        let report: mobench_sdk::RunnerReport = serde_json::from_slice(report_bytes).unwrap();
        assert_eq!(report.spec.name, "basic_benchmark::bench_checksum");
        assert_eq!(report.samples.len(), 2);

        unsafe { mobench_free_buf(&mut out) };
        assert!(out.ptr.is_null());
        assert_eq!(out.len, 0);
        assert_eq!(out.cap, 0);
    }

    #[test]
    fn test_unknown_function_maps_to_mobile_ffi_error() {
        let spec = BenchSpec {
            name: "basic_benchmark::does_not_exist".to_string(),
            iterations: 1,
            warmup: 0,
        };
        let result = run_benchmark(spec);
        assert!(matches!(result, Err(BenchError::UnknownFunction { .. })));
    }
}
