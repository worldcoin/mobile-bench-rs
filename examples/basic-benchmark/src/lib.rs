//! Basic benchmark examples demonstrating mobench-sdk usage.
//!
//! This example keeps things minimal: register functions with #[benchmark] and
//! let the SDK handle discovery and execution. See `examples/ffi-benchmark` for
//! a full UniFFI-based FFI surface.

use mobench_sdk::benchmark;

const CHECKSUM_INPUT: [u8; 1024] = [1; 1024];

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
    fn test_discover_benchmarks() {
        let benchmarks = mobench_sdk::discover_benchmarks();
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
}
