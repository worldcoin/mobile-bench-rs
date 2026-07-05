# mobench-sdk Integration Guide

Current release: **0.1.43**.

This guide shows how to add `mobench-sdk` to a Rust crate, register
benchmarks, configure the generated mobile runner backend, and run through the
mobench CLI.

## Add Dependencies

Full SDK:

```toml
[dependencies]
mobench-sdk = "0.1.43"
inventory = "0.3"

[lib]
crate-type = ["cdylib", "staticlib", "lib"]
```

Runtime-only benchmark crates can use the narrower registry feature:

```toml
[dependencies]
mobench-sdk = { version = "0.1.43", default-features = false, features = ["registry"] }
inventory = "0.3"

[lib]
crate-type = ["cdylib", "staticlib", "lib"]
```

Use `cdylib` for Android shared libraries, `staticlib` for iOS framework
slices, and `lib` for normal host tests.

## Write Benchmarks

Simple benchmarks take no parameters and return `()`:

```rust
use mobench_sdk::benchmark;

#[benchmark]
pub fn checksum_bench() {
    let data = [1u8; 1024];
    let sum: u64 = data.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

Use setup when expensive initialization should not be measured:

```rust
use mobench_sdk::benchmark;

fn create_test_data() -> Vec<u8> {
    vec![42; 1024 * 1024]
}

#[benchmark(setup = create_test_data)]
pub fn process_data(data: &Vec<u8>) {
    let sum: u64 = data.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

Use `per_iteration` when each measured iteration needs fresh input:

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

Use teardown when setup creates resources that must be cleaned up:

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

The macro validates signatures at compile time. Simple benchmarks must have no
parameters. Setup benchmarks must accept one input compatible with the setup
function return type.

## Configure mobench

Create `mobench.toml` at the repository root:

```toml
[project]
crate = "my-bench-crate"
library_name = "my_bench_crate"
ffi_backend = "uniffi" # default; also supports "native-c-abi"

[android]
package = "com.example.bench"
min_sdk = 24
target_sdk = 34

[ios]
bundle_id = "com.example.bench"
deployment_target = "15.0"

[benchmarks]
default_function = "my_bench_crate::checksum_bench"
default_iterations = 100
default_warmup = 10
```

`uniffi` is the compatibility backend. It generates Kotlin/Swift bindings for
mobile runners.

`native-c-abi` generates runners that call `mobench_run_benchmark_json`
directly. Opt in from the benchmark crate root:

```rust
mobench_sdk::export_native_c_abi!();
```

The native C ABI exports:

- `mobench_run_benchmark_json`
- `mobench_free_buf`
- `mobench_last_error_message`
- `MobenchBuf`

## Run Locally

List registered benchmarks:

```bash
cargo mobench list --crate-path crates/my-bench-crate
```

Run the host harness without mobile builds:

```bash
cargo mobench run \
  --target android \
  --function my_bench_crate::checksum_bench \
  --crate-path crates/my-bench-crate \
  --local-only \
  --iterations 100 \
  --warmup 10 \
  --output target/mobench/results.json
```

Inspect the report:

```bash
cargo mobench summary target/mobench/results.json
cargo mobench summary --format json target/mobench/results.json
cargo mobench summary --format csv target/mobench/results.json
```

## Build Mobile Artifacts

Check prerequisites before building:

```bash
cargo mobench check --target android
cargo mobench check --target ios
```

Build generated mobile projects:

```bash
cargo mobench build --target android --progress
cargo mobench build --target ios --progress
```

Build commands resolve the benchmark crate from explicit flags, `mobench.toml`,
Cargo workspace metadata, a git root, or the legacy `bench-mobile/` layout.
Outputs default to `target/mobench/`.

## BrowserStack Runs

Set credentials:

```bash
export BROWSERSTACK_USERNAME="your_username"
export BROWSERSTACK_ACCESS_KEY="your_access_key"
export BROWSERSTACK_PROJECT="mobile-benchmarks"
```

Run on a BrowserStack Android device:

```bash
cargo mobench run \
  --target android \
  --function my_bench_crate::checksum_bench \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --output target/mobench/results.json
```

Use `--release` for BrowserStack to keep Android APK uploads smaller.

## CI Contract

Use `ci run` for stable automation outputs:

```bash
cargo mobench ci run \
  --target android \
  --function my_bench_crate::checksum_bench \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --plots auto \
  --output-dir target/mobench/ci
```

The CI output directory includes:

- `summary.json`
- `summary.md`
- `results.csv`
- `plots/*.svg` when plot rendering is available and enabled

The schemas live in `docs/schemas/`.

## Semantic Profiling

Semantic phase annotations can be included in native profile summaries:

```rust
use mobench_sdk::{benchmark, profile_phase};

#[benchmark]
pub fn prove_and_verify() {
    let proof = profile_phase("prove", || prove());
    profile_phase("verify", || verify(&proof));
}
```

Run local native profiling separately from timing benchmarks:

```bash
cargo mobench profile run \
  --target android \
  --provider local \
  --backend android-native \
  --function my_bench_crate::prove_and_verify
```

BrowserStack native stack/flamegraph capture is not supported in this release.

## Public SDK Surface

Frequently used exports:

- `benchmark`
- `debug_benchmarks`
- `BenchmarkBuilder`
- `run_benchmark`
- `discover_benchmarks`
- `find_benchmark`
- `list_benchmark_names`
- `BenchSpec`, `BenchSample`, `BenchReport`, `BenchSummary`, `RunnerReport`
- `SemanticPhase`, `HarnessTimelineSpan`, `TimingError`
- `profile_phase`, `run_closure`
- `Target`, `FfiBackend`, `BuildConfig`, `BuildProfile`, `BuildResult`
- `MobenchBuf`
