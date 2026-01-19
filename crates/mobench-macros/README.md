# mobench-macros

Procedural macros for the [mobench](https://crates.io/crates/mobench) mobile benchmarking framework.

This crate provides the `#[benchmark]` attribute macro that automatically registers functions for mobile benchmarking. It uses compile-time registration via the `inventory` crate to build a registry of benchmark functions.

## Features

- **`#[benchmark]` attribute**: Mark functions as benchmarks
- **Automatic registration**: No manual registry maintenance required
- **Type safety**: Compile-time validation of benchmark functions
- **Zero runtime overhead**: Registration happens at compile time

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
mobench-macros = "0.1"
mobench-sdk = "0.1"  # For the runtime
```

### Basic Example

```rust
use mobench_macros::benchmark;

#[benchmark]
fn fibonacci_benchmark() {
    let result = fibonacci(30);
    std::hint::black_box(result);
}

#[benchmark]
fn sorting_benchmark() {
    let mut data = vec![5, 2, 8, 1, 9];
    data.sort();
    std::hint::black_box(data);
}

fn fibonacci(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}
```

### With mobench-sdk

The macros work seamlessly with mobench-sdk:

```rust
use mobench_macros::benchmark;
use mobench_sdk::{run_benchmark, BenchSpec};

#[benchmark]
fn my_expensive_operation() {
    // Your benchmark code
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Run the benchmark
    let spec = BenchSpec::new("my_expensive_operation", 100, 10)?;
    let report = run_benchmark(spec)?;

    println!("Mean: {} ns", report.mean_ns());
    Ok(())
}
```

## How It Works

The `#[benchmark]` macro:

1. **Preserves your function**: The original function remains unchanged
2. **Generates registration code**: Creates an `inventory::submit!` call
3. **Wraps in closure**: Converts your function into a callable closure
4. **Registers at compile time**: Adds to the global benchmark registry

### Macro Expansion

When you write:

```rust
#[benchmark]
fn my_benchmark() {
    expensive_operation();
}
```

The macro expands to something like:

```rust
fn my_benchmark() {
    expensive_operation();
}

inventory::submit! {
    BenchFunction {
        name: "my_benchmark",
        invoke: |_args| {
            my_benchmark();
            Ok(())
        }
    }
}
```

## Requirements

- Functions must be regular functions (not async)
- Functions should not take parameters
- Functions should use `std::hint::black_box()` to prevent optimization of results

## Best Practices

### Prevent Compiler Optimization

Always use `black_box` for benchmark results:

```rust
use mobench_macros::benchmark;

#[benchmark]
fn good_benchmark() {
    let result = expensive_computation();
    std::hint::black_box(result); // ✓ Prevents optimization
}

#[benchmark]
fn bad_benchmark() {
    let result = expensive_computation(); // ✗ May be optimized away
}
```

### Benchmark Naming

Use descriptive names that indicate what's being measured:

```rust
#[benchmark]
fn hash_1kb_data() { /* ... */ }

#[benchmark]
fn parse_json_small() { /* ... */ }

#[benchmark]
fn encrypt_aes_256() { /* ... */ }
```

### Isolate Benchmarks

Keep benchmarks focused on one operation:

```rust
// Good: Measures one thing
#[benchmark]
fn sha256_hash() {
    let hash = sha256(&DATA);
    std::hint::black_box(hash);
}

// Bad: Measures multiple things
#[benchmark]
fn hash_and_encode() {
    let hash = sha256(&DATA);
    let encoded = base64_encode(hash);
    std::hint::black_box(encoded);
}
```

## Part of mobench

This crate is part of the mobench ecosystem for mobile benchmarking:

- **[mobench](https://crates.io/crates/mobench)** - CLI tool for running benchmarks
- **[mobench-sdk](https://crates.io/crates/mobench-sdk)** - SDK with timing harness for integrating benchmarks
- **[mobench-macros](https://crates.io/crates/mobench-macros)** - This crate (proc macros)

## See Also

- [mobench-sdk](https://crates.io/crates/mobench-sdk) for the complete SDK
- [mobench](https://crates.io/crates/mobench) for the CLI tool
- [inventory](https://crates.io/crates/inventory) for the registration mechanism

## License

Licensed under the MIT License. See [LICENSE.md](../../LICENSE.md) for details.

Copyright (c) 2026 World Foundation
