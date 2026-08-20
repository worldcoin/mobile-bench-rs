# mobench-macros

Procedural macros for the [mobench](https://crates.io/crates/mobench) mobile
benchmarking toolkit.

This crate provides the `#[benchmark]` attribute macro. The macro preserves the
annotated Rust function, validates the benchmark signature at compile time, and
registers it through `inventory` so `mobench-sdk` and the `cargo mobench` CLI
can discover and run it.

Current release: **0.2.1**.

## Install

Most users should depend on `mobench-sdk`, which re-exports the macro:

```toml
[dependencies]
mobench-sdk = "0.2.0"
inventory = "0.3"
```

Direct macro use is also supported:

```toml
[dependencies]
mobench-macros = "0.2.0"
mobench-sdk = { version = "0.2.1", default-features = false, features = ["registry"] }
inventory = "0.3"
```

## Basic Example

```rust
use mobench_sdk::benchmark;

#[benchmark]
pub fn fibonacci_benchmark() {
    let result = fibonacci(30);
    std::hint::black_box(result);
}

fn fibonacci(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}
```

## Setup, Teardown, and Per-Iteration Input

Setup code runs outside measured timing:

```rust
use mobench_sdk::benchmark;

fn setup_data() -> Vec<u8> {
    vec![42; 1024 * 1024]
}

#[benchmark(setup = setup_data)]
pub fn checksum(data: &Vec<u8>) {
    let sum: u64 = data.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

Use `per_iteration` when the benchmark mutates or consumes its input:

```rust
use mobench_sdk::benchmark;

fn unsorted_vec() -> Vec<i32> {
    (0..1000).rev().collect()
}

#[benchmark(setup = unsorted_vec, per_iteration)]
pub fn sort_vec(mut data: Vec<i32>) {
    data.sort();
    std::hint::black_box(data);
}
```

Teardown can clean up resources after measured iterations:

```rust
use mobench_sdk::benchmark;

fn setup_db() -> Database {
    Database::connect("bench.db")
}

fn cleanup_db(db: Database) {
    db.close();
}

#[benchmark(setup = setup_db, teardown = cleanup_db)]
pub fn query(db: &Database) {
    db.query("SELECT 1");
}
```

## Signature Rules

- Simple benchmarks take no parameters and return `()`.
- Setup benchmarks take one parameter matching the setup function output.
- `per_iteration` setup passes the setup value by value.
- Non-`per_iteration` setup passes the setup value by reference.
- Benchmark functions should be `pub` when they need to be linked into generated
  mobile runners.

## How It Works

`#[benchmark]` expands to registration code similar to:

```rust
inventory::submit! {
    mobench_sdk::BenchFunction {
        name: "my_crate::my_benchmark",
        runner: |spec| {
            mobench_sdk::timing::run_closure(spec, || {
                my_benchmark();
                Ok(())
            })
        },
    }
}
```

The `cargo mobench list`, `verify`, `run`, and `ci run` commands use that
registry through the SDK runtime.

## License

MIT licensed, World Foundation 2026.
