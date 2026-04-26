# Examples

Updated: 2026-04-26

Use these examples to pick the smallest starting point for a mobench
integration.

## Minimal Benchmark Crate

Path: `examples/basic-benchmark`

Use this when you only need Rust benchmark functions and SDK registry execution.

```bash
cargo test -p basic-benchmark
cargo mobench list --crate-path examples/basic-benchmark
cargo mobench run --target android --function basic_benchmark::fibonacci --local-only
```

## Setup And Teardown Benchmarks

Use `#[benchmark(setup = setup_fn)]` when expensive fixture creation should not
be timed as benchmark work.

```rust
use mobench_sdk::benchmark;

struct ProofInput {
    bytes: Vec<u8>,
}

fn setup_proof() -> ProofInput {
    ProofInput { bytes: vec![42; 4096] }
}

#[benchmark(setup = setup_proof)]
fn verify_proof(input: &ProofInput) {
    std::hint::black_box(input.bytes.len());
}
```

Use per-iteration setup when each measured iteration must own fresh input:

```rust
use mobench_sdk::benchmark;

fn setup_message() -> Vec<u8> {
    vec![7; 1024]
}

#[benchmark(setup_per_iter = setup_message)]
fn hash_message(mut message: Vec<u8>) {
    message.reverse();
    std::hint::black_box(message);
}
```

## FFI And Custom Types

Path: `examples/ffi-benchmark`

Use this when your benchmark crate already exposes UniFFI bindings or needs
custom FFI-facing request/error types.

```bash
cargo test -p ffi-benchmark
cargo mobench list --crate-path examples/ffi-benchmark
cargo mobench run --target android --function ffi_benchmark::fibonacci --local-only
```

## CI-Only Benchmark Workflow

Use `ci run` when a workflow should produce stable machine-readable outputs.

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --local-only \
  --plots auto
```

Outputs are written under `target/mobench/ci/`:

- `summary.json`
- `summary.md`
- `results.csv`
- `plots/*.svg` when plot rendering is enabled

## Profiling Workflow

Use local profiling when you need native stack artifacts and flamegraphs.

```bash
cargo mobench profile run \
  --target android \
  --provider local \
  --backend android-native \
  --crate-path crates/sample-fns \
  --function sample_fns::fibonacci

cargo mobench profile summarize \
  --profile target/mobench/profile/profile.json
```

## Programmatic SDK Usage

Use the SDK directly when embedding mobench into another Rust tool.

```rust
use mobench_sdk::{BenchSpec, run_benchmark};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = BenchSpec::new("sample_fns::fibonacci", 100, 10)?;
    let report = run_benchmark(spec)?;

    println!("median: {} ns", report.median_ns());
    Ok(())
}
```
