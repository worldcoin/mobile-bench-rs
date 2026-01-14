# mobench-runner

Lightweight benchmarking harness for mobile devices.

This crate provides the core timing infrastructure for running benchmarks on mobile platforms (Android and iOS). It's designed to be embedded in mobile apps and provides accurate timing measurements with configurable iterations and warmup cycles.

## Features

- **Accurate timing**: High-precision timing measurements for benchmarks
- **Configurable**: Set iterations and warmup cycles
- **Mobile-optimized**: Designed for resource-constrained mobile environments
- **Serializable results**: Results can be serialized with serde for transmission to host
- **No dependencies**: Minimal dependencies for fast compilation

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
mobench-runner = "0.1"
```

### Basic Example

```rust
use mobench_runner::{BenchSpec, run_closure};

// Create a benchmark specification
let spec = BenchSpec {
    name: "my_benchmark".to_string(),
    iterations: 100,
    warmup: 10,
};

// Run a closure as a benchmark
let report = run_closure(spec, || {
    // Your benchmark code here
    let result = expensive_computation();
    std::hint::black_box(result); // Prevent optimization
    Ok(())
})?;

// Access timing results
println!("Mean: {} ns", report.mean_ns());
println!("Median: {} ns", report.median_ns());
println!("Min: {} ns", report.min_ns());
println!("Max: {} ns", report.max_ns());
```

### With Error Handling

```rust
use mobench_runner::{BenchSpec, BenchError, run_closure};

fn my_benchmark() -> Result<(), String> {
    // Your code that might fail
    Ok(())
}

let spec = BenchSpec::new("my_benchmark", 50, 5)?;

let report = run_closure(spec, || {
    my_benchmark().map_err(|e| BenchError::Execution(e))
})?;
```

## Types

### `BenchSpec`

Specification for a benchmark run:

```rust
pub struct BenchSpec {
    pub name: String,      // Benchmark name
    pub iterations: u32,   // Number of iterations to run
    pub warmup: u32,       // Number of warmup iterations
}
```

### `BenchReport`

Results from a benchmark run:

```rust
pub struct BenchReport {
    pub spec: BenchSpec,
    pub samples: Vec<BenchSample>,
}
```

Provides helper methods:
- `mean_ns()` - Mean execution time
- `median_ns()` - Median execution time
- `min_ns()` - Minimum execution time
- `max_ns()` - Maximum execution time
- `stddev_ns()` - Standard deviation

### `BenchSample`

Individual timing sample:

```rust
pub struct BenchSample {
    pub duration_ns: u64,  // Duration in nanoseconds
}
```

## Use Cases

This crate is typically used as a dependency in larger benchmarking systems:

1. **Mobile benchmark harness**: Embed in mobile apps to run benchmarks on real devices
2. **Cross-platform timing**: Consistent timing API across platforms
3. **Remote benchmarking**: Serialize results and send to analysis tools

## Part of mobench

This crate is part of the [mobench](https://crates.io/crates/mobench) ecosystem for mobile benchmarking:

- **[mobench](https://crates.io/crates/mobench)** - CLI tool for running benchmarks
- **[mobench-sdk](https://crates.io/crates/mobench-sdk)** - SDK for integrating benchmarks
- **[mobench-macros](https://crates.io/crates/mobench-macros)** - Proc macros for `#[benchmark]`
- **[mobench-runner](https://crates.io/crates/mobench-runner)** - This crate (timing harness)

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)

at your option.
