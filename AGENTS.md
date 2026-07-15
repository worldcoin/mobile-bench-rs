# AGENTS.md

This file provides guidance to Codex and other coding agents when working in
this repository.

## Project Overview

mobile-bench-rs, now published as **mobench**, is a Rust mobile benchmarking
toolkit. It lets benchmark authors define Rust benchmarks once, build generated
Android/iOS runners, run host-only/local/BrowserStack benchmark jobs, produce
stable CI artifacts, and run local native profiling sessions.

Current workspace and crates.io release line: **v0.1.43**.

Published crates:

- [`mobench`](https://crates.io/crates/mobench): CLI and programmatic
  orchestration API.
- [`mobench-sdk`](https://crates.io/crates/mobench-sdk): timing harness,
  benchmark registry, generated runner support, Android/iOS builders, profiling
  helpers, UniFFI compatibility, and native C ABI support.
- [`mobench-macros`](https://crates.io/crates/mobench-macros): `#[benchmark]`
  proc macro.

All packages are MIT licensed by World Foundation, 2026.

## Commit Guidelines

Do not add `Co-Authored-By` lines to commit messages.

## Workspace Structure

```text
crates/mobench/          CLI, BrowserStack, reports, profiling
crates/mobench-sdk/      timing, registry, builders, codegen, templates
crates/mobench-macros/   #[benchmark] proc macro
crates/sample-fns/       repository demo benchmark crate
examples/basic-benchmark minimal SDK example
examples/ffi-benchmark   full generated FFI example
android/                 checked-in Android runner/demo app
ios/                     checked-in iOS runner/demo app
templates/               editable template sources
docs/                    guides, specs, schemas, diagrams, codebase reference
```

The workspace uses Rust 2024 with MSRV Rust 1.85.

## Main Product Surfaces

### Benchmark Execution

- Build Android/iOS artifacts.
- Run benchmarks host-only, locally, or on BrowserStack.
- Write result JSON, `summary.json`, `summary.md`, `results.csv`, optional
  `plots/*.svg`, PR comments, and check-run summaries.

### Local Native Profiling

- `android-native`: local Android `simpleperf` capture and symbolization.
- `ios-instruments`: local simulator-host `sample` capture.
- `rust-tracing`: planned manifest/trace contract.
- BrowserStack native stack/flamegraph profiling is explicitly unsupported in
  this release.

## Quick Start

```bash
cargo install mobench
cargo add mobench-sdk inventory
```

```rust
use mobench_sdk::benchmark;

#[benchmark]
pub fn my_benchmark() {
    let result = expensive_operation();
    std::hint::black_box(result);
}
```

Run a host-only smoke benchmark:

```bash
cargo mobench run \
  --target android \
  --function my_crate::my_benchmark \
  --local-only \
  --iterations 20 \
  --warmup 5 \
  --output target/mobench/results.json
```

## Common Commands

```bash
# Rust tests
cargo test --workspace

# CLI help and version
cargo run -q -p mobench --bin mobench -- --version
cargo run -q -p mobench --bin mobench -- --help

# Prerequisite checks
cargo mobench check --target android
cargo mobench check --target ios
cargo mobench doctor --target both --browserstack false

# Build mobile artifacts
cargo mobench build --target android --progress
cargo mobench build --target ios --progress
cargo mobench build --target both --progress

# List and verify benchmarks
cargo mobench list --crate-path examples/basic-benchmark
cargo mobench verify --target android --check-artifacts

# BrowserStack device resolution
cargo mobench devices --platform android
cargo mobench devices resolve \
  --platform android \
  --profile default \
  --device-matrix device-matrix.yaml

# CI contract output
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --local-only \
  --plots auto \
  --output-dir target/mobench/ci

# Fetch BrowserStack artifacts
cargo mobench fetch \
  --target android \
  --build-id <browserstack-build-id> \
  --output-dir target/browserstack \
  --wait

# Local native profiling
cargo mobench profile run \
  --target android \
  --provider local \
  --backend android-native \
  --function sample_fns::fibonacci
```

## Benchmark Authoring

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

Setup runs outside measured iterations:

```rust
fn create_input() -> Vec<u8> {
    vec![42; 1024 * 1024]
}

#[benchmark(setup = create_input)]
pub fn checksum(input: &Vec<u8>) {
    let sum: u64 = input.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

Use `per_iteration` when each measured sample needs fresh input:

```rust
fn unsorted_vec() -> Vec<i32> {
    (0..1000).rev().collect()
}

#[benchmark(setup = unsorted_vec, per_iteration)]
pub fn sort_vec(mut data: Vec<i32>) {
    data.sort();
    std::hint::black_box(data);
}
```

Use teardown when setup resources must be cleaned up:

```rust
#[benchmark(setup = setup_db, teardown = cleanup_db)]
pub fn query(db: &Database) {
    db.query("SELECT 1");
}
```

The macro validates supported signatures at compile time.

## Generated Runner Backends

Select generated runner backend in `mobench.toml`:

```toml
[project]
crate = "my-bench-crate"
library_name = "my_bench_crate"
ffi_backend = "uniffi" # or "native-c-abi"
```

- `uniffi`: compatibility default using generated Kotlin/Swift bindings.
- `native-c-abi`: generated runners call the mobench JSON C ABI directly.

For `native-c-abi`, export the ABI from the benchmark crate root:

```rust
mobench_sdk::export_native_c_abi!();
```

Exported symbols:

- `mobench_run_benchmark_json`
- `mobench_free_buf`
- `mobench_last_error_message`
- `MobenchBuf`

## Configuration And Resolution

Project resolution order for build/run/list/verify/package flows:

1. Explicit `--project-root`.
2. Explicit `--crate-path`.
3. Explicit `--config`.
4. Discovered `mobench.toml`.
5. Cargo workspace metadata.
6. Git root.
7. Legacy `bench-mobile/` fallback.

BrowserStack credentials resolve from config, environment variables, and
`.env.local`:

- `BROWSERSTACK_USERNAME`
- `BROWSERSTACK_ACCESS_KEY`
- `BROWSERSTACK_PROJECT`

## Android And iOS Notes

Android targets:

- `aarch64-linux-android`
- `armv7-linux-androideabi`
- `x86_64-linux-android`

Use `UNIFFI_ANDROID_ABI=x86_64` when testing the UniFFI Android path on a
default x86_64 emulator.

iOS targets:

- `aarch64-apple-ios`
- `aarch64-apple-ios-sim`
- `x86_64-apple-ios`

iOS BrowserStack packaging:

```bash
cargo mobench package-ipa --method adhoc
cargo mobench package-xcuitest
```

The iOS builder creates an xcframework under `target/mobench/ios/` and attempts
ad-hoc signing:

```bash
codesign --force --deep --sign - target/mobench/ios/<library_name>.xcframework
```

## CI Outputs

`cargo mobench ci run` writes:

- `summary.json`
- `summary.md`
- `results.csv`
- `plots/*.svg` when plot rendering is available and enabled

Important CSV/resource columns:

- `mean_ns`, `median_ns`, `p95_ns`, `min_ns`, `max_ns`
- `cpu_total_ms`, `cpu_median_ms`
- `peak_memory_kb`, `peak_memory_growth_kb`
- `process_peak_memory_kb`

## Profiling Artifacts

`cargo mobench profile run` writes run-scoped output under
`target/mobench/profile/<run-id>/` and latest convenience copies under
`target/mobench/profile/`.

Common files:

- `profile.json`
- `summary.md`
- `artifacts/raw/...`
- `artifacts/processed/stacks.folded`
- `artifacts/processed/native-report.txt`
- `artifacts/processed/frame-locations.json` on Android when available
- `artifacts/processed/flamegraph.full.svg`
- `artifacts/processed/flamegraph.focused.svg`
- `artifacts/processed/flamegraph.html`
- `artifacts/semantic/phases.json` when semantic phase data exists

## Public API Reference Points

Important SDK exports:

- `benchmark`, `debug_benchmarks`
- `BenchmarkBuilder`, `run_benchmark`
- `discover_benchmarks`, `find_benchmark`, `list_benchmark_names`
- `BenchSpec`, `BenchSample`, `BenchReport`, `BenchSummary`, `RunnerReport`
- `SemanticPhase`, `HarnessTimelineSpan`, `TimingError`
- `profile_phase`, `run_closure`
- `Target`, `FfiBackend`, `BuildConfig`, `BuildProfile`, `BuildResult`
- `MobenchBuf`

Important CLI crate exports:

- `RunRequest`
- `RunResult`
- `DeviceSelection`
- `Report`
- `run_request`

## Documentation Map

- `README.md`: user-facing project overview.
- `docs/guides/README.md`: current user guides.
- `docs/codebase/README.md`: codebase reference set.
- `docs/specs/mobench-current-spec.md`: current behavior and API spec.
- `docs/schemas/`: machine-readable output contracts.
- `docs/diagrams/`: Mermaid source diagrams mirrored in the README.
- `RELEASE_NOTES.md`: release history and support status.

## Working Conventions

- Prefer existing repo patterns over new abstractions.
- Keep generated template sources and embedded SDK templates in sync:
  `templates/` and `crates/mobench-sdk/templates/`.
- Keep BrowserStack native profiling unsupported unless implementation and docs
  are updated together.
- Update docs and schemas when output contracts change.
- Use `cargo mobench --help` and subcommand help to verify CLI docs.
