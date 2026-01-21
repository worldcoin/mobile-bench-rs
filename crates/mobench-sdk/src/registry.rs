//! Benchmark function registry
//!
//! This module provides runtime discovery of benchmark functions that have been
//! marked with the `#[benchmark]` attribute macro.

use crate::timing::{BenchReport, BenchSpec, TimingError};

/// A registered benchmark function
///
/// This struct is submitted to the global registry by the `#[benchmark]` macro.
/// It contains the function's name and a runner that executes the benchmark.
pub struct BenchFunction {
    /// Fully-qualified name of the benchmark function (e.g., "my_crate::my_module::my_bench")
    pub name: &'static str,

    /// Runner function that executes the benchmark with timing
    ///
    /// Takes a BenchSpec and returns a BenchReport directly.
    /// The runner handles setup/teardown internally.
    pub runner: fn(BenchSpec) -> Result<BenchReport, TimingError>,
}

// Register the BenchFunction type with inventory
inventory::collect!(BenchFunction);

/// Discovers all registered benchmark functions
///
/// Returns a vector of references to all functions that have been marked with
/// the `#[benchmark]` attribute in the current binary.
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::registry::discover_benchmarks;
///
/// let benchmarks = discover_benchmarks();
/// for bench in benchmarks {
///     println!("Found benchmark: {}", bench.name);
/// }
/// ```
pub fn discover_benchmarks() -> Vec<&'static BenchFunction> {
    inventory::iter::<BenchFunction>().collect()
}

/// Finds a benchmark function by name
///
/// Searches the registry for a function with the given name. Supports both
/// short names (e.g., "fibonacci") and fully-qualified names
/// (e.g., "my_crate::fibonacci").
///
/// # Arguments
///
/// * `name` - The name of the benchmark to find
///
/// # Returns
///
/// * `Some(&BenchFunction)` if found
/// * `None` if no matching benchmark exists
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::registry::find_benchmark;
///
/// if let Some(bench) = find_benchmark("fibonacci") {
///     println!("Found benchmark: {}", bench.name);
/// } else {
///     eprintln!("Benchmark not found");
/// }
/// ```
pub fn find_benchmark(name: &str) -> Option<&'static BenchFunction> {
    inventory::iter::<BenchFunction>().find(|f| {
        // Match either the full name or just the final component
        f.name == name || f.name.ends_with(&format!("::{}", name))
    })
}

/// Lists all registered benchmark names
///
/// Returns a sorted vector of all benchmark function names in the registry.
///
/// # Example
///
/// ```ignore
/// use mobench_sdk::registry::list_benchmark_names;
///
/// let names = list_benchmark_names();
/// println!("Available benchmarks:");
/// for name in names {
///     println!("  - {}", name);
/// }
/// ```
pub fn list_benchmark_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = inventory::iter::<BenchFunction>().map(|f| f.name).collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_benchmarks() {
        // Note: This test validates that the discovery function works
        // The number of benchmarks depends on what's registered in the binary
        let benchmarks = discover_benchmarks();
        // Just ensure the function returns successfully
        let _ = benchmarks;
    }

    #[test]
    fn test_find_benchmark_none() {
        // Should not find a non-existent benchmark
        let result = find_benchmark("nonexistent_benchmark_function_12345");
        assert!(result.is_none());
    }

    #[test]
    fn test_list_benchmark_names() {
        // Validates that the function returns successfully
        let names = list_benchmark_names();
        let _ = names;
    }
}
