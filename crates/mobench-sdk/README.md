# mobench-sdk

Rust SDK for mobench mobile benchmarking. It provides the benchmark timing
harness, `#[benchmark]` registry integration, generated runner support, Android
and iOS builders, UniFFI compatibility, native JSON C ABI exports, and semantic
profiling helpers used by the `mobench` CLI.

Current release: **0.1.44**.

## Features

- `#[benchmark]` re-export from `mobench-macros`.
- Inventory-backed benchmark discovery and runtime execution.
- Lightweight timing harness with warmup and measured iterations.
- Setup, teardown, and per-iteration benchmark inputs.
- Android and iOS build automation used by `cargo mobench build/run/ci run`.
- Generated mobile runner templates for:
  - `uniffi` (default compatibility backend)
  - `native-c-abi` (direct mobench JSON C ABI backend)
- `profile_phase(...)` semantic phase annotations for native profile summaries.
- Stable serializable timing/report types for mobile and CI integrations.

## Install

Full SDK, including builders and code generation:

```toml
[dependencies]
mobench-sdk = "0.1.44"
inventory = "0.3"
```

Runtime-only benchmark crates that do not call SDK builders directly can use the
narrower registry feature:

```toml
[dependencies]
mobench-sdk = { version = "0.1.44", default-features = false, features = ["registry"] }
inventory = "0.3"
```

For generated mobile libraries, configure crate types:

```toml
[lib]
crate-type = ["cdylib", "staticlib", "lib"]
```

## Basic Benchmark

```rust
use mobench_sdk::benchmark;

#[benchmark]
pub fn fibonacci_30() {
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

Run the benchmark through the CLI:

```bash
cargo install mobench
cargo mobench list
cargo mobench run --target android --function fibonacci_30 --local-only
```

## Programmatic Runtime

```rust
use mobench_sdk::{run_benchmark, BenchSpec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = BenchSpec::new("fibonacci_30", 100, 10)?;
    let report = run_benchmark(spec)?;

    println!("mean: {} ns", report.mean_ns());
    println!("median: {} ns", report.median_ns());
    Ok(())
}
```

## Setup and Teardown

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

Use `per_iteration` for mutable or consumed input:

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

## Generated Runner Backends

Generated mobile runners read backend configuration from `mobench.toml`:

```toml
[project]
crate = "my-bench-crate"
library_name = "my_bench_crate"
ffi_backend = "uniffi" # default; also supports "native-c-abi" and "boltffi"

[ios]
deployment_target = "15.0"
# runner = "swiftui" # or "uikit-legacy" for legacy iOS targets
```

Use `uniffi` for the historical generated Kotlin/Swift binding path.

Use `native-c-abi` when you want generated Android and iOS runners to call
mobench's direct JSON C ABI and avoid UniFFI binding-generation overhead in the
measured path. Native C ABI benchmark crates should export the ABI from the
crate root:

```rust
mobench_sdk::export_native_c_abi!();
```

This exports:

- `mobench_run_benchmark_json`
- `mobench_free_buf`
- `mobench_last_error_message`
- `MobenchBuf`

Use `boltffi` when you want generated Android and iOS runners to call
BoltFFI-generated Kotlin/Swift bindings. Benchmark crates should export a
`run_benchmark_json(spec_json: &str) -> Result<String, String>` function with
`#[boltffi::export]`.

iOS deployment targets below 15.0 select the UIKit legacy runner by default.
Forcing `runner = "swiftui"` below 15.0 fails early. Legacy BrowserStack
targets such as iPhone 7 on iOS 10 also require an older Xcode lane capable of
building for that OS.

## Semantic Profiling Phases

Native profile runs can include semantic phase timings:

```rust
use mobench_sdk::{benchmark, profile_phase};

#[benchmark]
pub fn prove_and_verify() {
    let proof = profile_phase("prove", || prove());
    profile_phase("verify", || verify(&proof));
}
```

`cargo mobench profile run` merges these phases into the profile manifest and
summary while keeping native stack flamegraphs separate from semantic timing.

## Build Through The CLI

The SDK builders are primarily driven by the CLI:

```bash
cargo mobench check --target android
cargo mobench check --target ios
cargo mobench build --target android --progress
cargo mobench build --target ios --progress
cargo mobench ci run --target android --function fibonacci_30 --local-only
```

Build/run/list/verify/package commands resolve the benchmark crate and project
root from explicit flags, `mobench.toml`, Cargo workspace metadata, git root, or
the legacy `bench-mobile/` layout.

## Important Re-Exports

- `benchmark`
- `debug_benchmarks`
- `BenchmarkBuilder`
- `run_benchmark`
- `discover_benchmarks`
- `find_benchmark`
- `list_benchmark_names`
- `BenchSpec`, `BenchSample`, `RunnerReport`, `BenchSummary`
- `SemanticPhase`, `HarnessTimelineSpan`, `TimingError`
- `profile_phase`, `run_closure`
- `Target`, `FfiBackend`, `BuildConfig`, `BuildProfile`, `BuildResult`
- `MobenchBuf`

## License

MIT licensed, World Foundation 2026.
