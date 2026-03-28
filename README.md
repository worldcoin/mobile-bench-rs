<p align="center">
  <img src="assets/mobench.jpg" width="280" alt="mobench" />
</p>

# mobench

Mobile benchmarking toolkit for Rust. Build and run Rust benchmarks on Android and iOS, locally or on BrowserStack, with a library-first workflow, config-first project resolution, and local native profiling that produces interactive flamegraph artifacts.

## What it is

mobench provides a Rust API and a CLI for running benchmarks on real mobile devices. You define benchmarks in Rust, generate mobile bindings automatically, and drive execution from the CLI with consistent output formats (JSON, Markdown, CSV).

For programmatic CI integrations, `mobench` exposes typed request/result types (`RunRequest`, `RunResult`, `DeviceSelection`, `Report`) via the crate API.

## How mobench works

- `#[benchmark]` marks functions and registers them via `inventory`
- `mobench-sdk` builds mobile artifacts, provides the timing harness, and generates app templates from embedded assets
- UniFFI proc macros generate Kotlin and Swift bindings directly from Rust types
- The CLI writes a benchmark spec (function, iterations, warmup) and packages it into the app
- Mobile apps call `run_benchmark` via the generated bindings and return timing samples
- The CLI collects results locally or from BrowserStack and writes summaries

## Workspace crates

- `crates/mobench` ([mobench](https://crates.io/crates/mobench)): CLI tool that builds, runs, and fetches benchmarks
- `crates/mobench-sdk` ([mobench-sdk](https://crates.io/crates/mobench-sdk)): core SDK with timing harness, builders, registry, and codegen
- `crates/mobench-macros` ([mobench-macros](https://crates.io/crates/mobench-macros)): `#[benchmark]` proc macro
- `crates/sample-fns`: sample benchmarks and UniFFI bindings
- `examples/basic-benchmark`: minimal SDK integration example
- `examples/ffi-benchmark`: full UniFFI/FFI surface example

## Quick start

```bash
# Install the CLI (fast)
cargo binstall mobench

# Or build from source
cargo install mobench

# Add the SDK to your project
cargo add mobench-sdk inventory

# Check prerequisites before building
cargo mobench doctor --target both
cargo mobench config validate --config bench-config.toml
cargo mobench check --target android
cargo mobench check --target ios

# Build artifacts (outputs to target/mobench/ by default)
cargo mobench build --target android
cargo mobench build --target ios

# Build with progress output for clearer feedback
cargo mobench build --target android --progress

# Run a benchmark locally
cargo mobench run --target android --function sample_fns::fibonacci

# Run on BrowserStack (use --release for smaller APK uploads)
cargo mobench run --target android --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" --release

# List available BrowserStack devices
cargo mobench devices --platform android

# Resolve matrix devices deterministically for CI
cargo mobench devices resolve --platform android --profile default --device-matrix device-matrix.yaml

# Fixture lifecycle helpers
cargo mobench fixture init
cargo mobench fixture verify
cargo mobench fixture cache-key

# View benchmark results summary
cargo mobench summary target/mobench/results.json

# CI one-command orchestration with stable outputs
cargo mobench ci run --target android --function sample_fns::fibonacci --local-only --plots auto

# Reporting helpers from standardized outputs
cargo mobench report summarize --summary target/mobench/ci/summary.json --plots auto
cargo mobench report github --pr 123 --summary target/mobench/ci/summary.json

# Local native profiling
cargo mobench profile run --target android --function sample_fns::fibonacci \
  --provider local --backend android-native
cargo mobench profile summarize --profile target/mobench/profile/profile.json
```

CI contract outputs are written to `target/mobench/ci/`:
- `summary.json`
- `summary.md`
- `results.csv`
- `plots/*.svg` when local plot rendering is enabled

Local summary renderers (`ci run --plots ...` and `report summarize --plots ...`) append a `Device Comparison Plots` section with one Sina-style SVG per benchmark function. Summary resource fields use `cpu_total_ms` and `peak_memory_kb`; Android raw resource stats are preserved and iOS peak memory is enriched from BrowserStack app profiling when available.

Profiling commands are local-first in this release. Each session
writes its current manifest and summary under
`target/mobench/profile/<run-id>/`, and the CLI also refreshes top-level
`target/mobench/profile/profile.json` and `summary.md` as convenience copies of
the latest run.

The manifest is split into three explicit sections:

- `native_capture`: native stack artifacts, symbolization state, and viewer hints
- `semantic_profile`: optional benchmark phase data such as `prove` and `serialize`
- `capture_metadata`: device resolution, capture settings, and warnings

The summary renderer keeps native and semantic outputs separate so the
interactive flamegraph viewer stays focused on native stacks while phase
timings remain readable as benchmark metadata.

When a benchmark uses `mobench_sdk::timing::profile_phase(...)`, local profile
runs also persist a run-scoped semantic sidecar at
`artifacts/semantic/phases.json`. The profile summary renders those phase totals
separately from the flamegraph so phase timing does not get mislabeled as native
stack data.

Profiling capability matrix:

| Provider | Backend | Current behavior | Notes |
|----------|---------|------------------|-------|
| `local` | `android-native` | Attempts real native capture | Uses `simpleperf`, symbolized `stacks.folded`, `native-report.txt`, `flamegraph.html`, and semantic phase summaries when the benchmark emits `profile_phase` data and an `adb` device is available |
| `local` | `ios-instruments` | Attempts real native capture | Uses a simulator-host `sample` capture to write `sample.txt`, `stacks.folded`, `native-report.txt`, and `flamegraph.html`. Semantic phase summaries are merged when the benchmark JSON includes `phases`. |
| `local` | `rust-tracing` | Planned manifest only | Structured trace output is local-only and still not implemented |
| `browserstack` | `android-native` | Unsupported | Use `--provider local` for planning/local capture, or a normal BrowserStack benchmark for timing/memory metrics |
| `browserstack` | `ios-instruments` | Unsupported | Use `--provider local` for simulator-host `sample` capture and flamegraphs. BrowserStack does not provide retrievable native iOS profile artifacts in this release. |
| `browserstack` | `rust-tracing` | Unsupported | Use `--provider local` for trace-events output |

For local native profiling, `profile run` also accepts `--warmup-mode warm|cold`.
Warm mode is the default for local Android/iOS native plans. On Android it performs
one preparatory launch before recording to prime startup caches and reduce first-run
noise. That improves the capture, but it does not remove all per-process bridge
initialization from the recorded run.

When you need device-specific planning inputs for profiling, `profile run`
reuses the same resolution model as `devices resolve`:

- `--device "iPhone 14" --os-version 16`
- `--profile high-spec`
- `--profile high-spec --device-matrix device-matrix.yaml`

## Configuration

mobench supports a `mobench.toml` configuration file for project settings:

```toml
[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"

[android]
package = "com.example.bench"
min_sdk = 24

[ios]
bundle_id = "com.example.bench"
deployment_target = "15.0"

[benchmarks]
default_function = "my_crate::my_benchmark"
default_iterations = 100
default_warmup = 10
```

Resolution precedence is: explicit CLI flags (`--project-root`, `--crate-path`) → explicit `--config` → discovered `mobench.toml` → Cargo workspace root → git root → legacy `bench-mobile` fallback.

CLI flags override config file values when provided.
- In `cargo mobench run --config <FILE>` mode, `--device-matrix <FILE>` overrides `device_matrix` from the config file.
- For regression comparisons, `--baseline` should point to a previous run summary; if it resolves to the same output path, mobench snapshots the prior file before writing the candidate summary.
- In the reusable GitHub workflow, the default baseline source is the latest successful run on the repository default branch when matching artifacts are available.
- `cargo mobench verify --smoke-test` is only supported for benchmark crates linked into the `mobench` CLI binary. External crates discovered through `mobench.toml`, `--project-root`, or `--crate-path` should use `cargo mobench list` and `cargo mobench verify --check-artifacts`.

## Project docs

- `docs/codebase/README.md`: current codebase reference map
- `BENCH_SDK_INTEGRATION.md`: SDK integration guide
- `BUILD.md`: build prerequisites and troubleshooting
- `TESTING.md`: testing guide and device workflows
- `BROWSERSTACK_CI_INTEGRATION.md`: BrowserStack CI setup
- `docs/CONTRACT_CI_V1.md`: frozen v1 CI input/output/error contract
- `docs/adr/0001-mobench-ci-contract-v1.md`: CI contract ADR and compatibility policy
- `docs/schemas/`: machine-readable CI/summary schema artifacts
- `docs/MIGRATION_GUIDE.md`: migration guide (placeholder, linked from ADR)
- `FETCH_RESULTS_GUIDE.md`: fetching and summarizing results
- `PROJECT_PLAN.md`: goals and backlog
- `CLAUDE.md`: developer guide

## Setup and Teardown

For benchmarks that require expensive setup (like generating test data or initializing connections), you can exclude setup time from measurements using the `setup` attribute.

### The Problem

Without setup/teardown, expensive initialization is measured as part of your benchmark:

```rust
#[benchmark]
fn verify_proof() {
    let proof = generate_complex_proof();  // This is measured (bad!)
    verify(&proof);                         // This is what we want to measure
}
```

### The Solution

Use the `setup` attribute to run initialization once before timing begins:

```rust
// Setup function runs once before all iterations (not timed)
fn setup_proof() -> ProofInput {
    generate_complex_proof()  // Takes 5 seconds, but not measured
}

#[benchmark(setup = setup_proof)]
fn verify_proof(input: &ProofInput) {
    verify(&input.proof);  // Only this is measured
}
```

### Per-Iteration Setup

For benchmarks that mutate their input, use `per_iteration` to get fresh data each iteration:

```rust
fn generate_random_vec() -> Vec<i32> {
    (0..1000).map(|_| rand::random()).collect()
}

#[benchmark(setup = generate_random_vec, per_iteration)]
fn sort_benchmark(data: Vec<i32>) {
    let mut data = data;
    data.sort();  // Each iteration gets a fresh unsorted vec
}
```

### Setup with Teardown

For resources that need cleanup (database connections, temp files, etc.):

```rust
fn setup_db() -> Database { Database::connect("test.db") }
fn cleanup_db(db: Database) { db.close(); std::fs::remove_file("test.db").ok(); }

#[benchmark(setup = setup_db, teardown = cleanup_db)]
fn db_query(db: &Database) {
    db.query("SELECT * FROM users");
}
```

### When to Use Each Pattern

| Pattern | Use Case |
|---------|----------|
| `#[benchmark]` | Simple benchmarks with no setup or fast inline setup |
| `#[benchmark(setup = fn)]` | Expensive one-time setup, reused across iterations |
| `#[benchmark(setup = fn, per_iteration)]` | Benchmarks that mutate input, need fresh data each time |
| `#[benchmark(setup = fn, teardown = fn)]` | Resources requiring cleanup (connections, files, etc.) |

## Release Notes

### v0.1.25

- Clarified that profiling remains local-first in this release; BrowserStack native profiling is explicitly unsupported with actionable error text and a visible capability matrix.
- Split `profile run` into target resolution, capture planning, and capture execution seams so planned manifests no longer imply that native capture actually ran.
- Added device-selection inputs to `profile run` (`--device`, `--os-version`, `--profile`, `--device-matrix`) by reusing the existing deterministic device-resolution flow.
- Added real local iOS native capture via simulator-host `sample`, with `sample.txt`, `stacks.folded`, `native-report.txt`, and `flamegraph.html` written into the normalized profile session layout.
- Added regression coverage for profile help text, BrowserStack unsupported execution, dry-run planning semantics, and direct device target resolution.
- Added `cargo mobench profile run|summarize` commands for a normalized local profiling session contract across Android and iOS.
- Added the interactive dual-view flamegraph viewer plus full/focused SVG artifacts for local native profile runs.
- Profile sessions now write run-scoped artifacts under `target/mobench/profile/<run-id>/` and refresh top-level latest-session `profile.json` and `summary.md` convenience files.
- Profile manifests now preserve the selected provider and requested output format, and the CLI rejects unsupported format/backend combinations explicitly instead of silently planning the wrong artifacts.
- Updated the profiling smoke-test docs to use working `cargo run -p mobench --bin mobench -- ...` invocations from the repo root.
- Stabilized the SDK timing test suite by removing a timer-resolution assumption from the noop benchmark test.

### v0.1.24

- Switched BrowserStack device discovery to the unified `app-automate/devices.json` inventory for Android, iOS, and combined device listing.
- Filtered unified BrowserStack inventory results locally by OS so Espresso resolution stays Android-only and XCUITest resolution stays iOS-only.
- Added regression coverage for mixed Android+iOS BrowserStack inventories used by device-resolution commands.

### v0.1.23

- Added Sina-style per-function device comparison plots to local summaries:
  - `cargo mobench ci run --plots <auto|off|require>`
  - `cargo mobench report summarize --plots <auto|off|require>`
- Rendered one SVG plot per benchmark function in the `Device Comparison Plots` section of local markdown summaries.
- Switched summary resource reporting to `cpu_total_ms` and `peak_memory_kb`, and preserved BrowserStack-derived peak memory while backfilling CPU from raw benchmark results.
- Enabled BrowserStack app profiling on Android and iOS runs, including App Profiling v2 parsing for iOS peak-memory enrichment.
- Added baseline artifact download in the reusable CI workflow so `ci check-run` can compare PR results against the latest successful default-branch run.

### v0.1.22

- Fixed BrowserStack result fetching so `cargo mobench ci run --fetch` falls back to downloaded session artifacts when live device logs do not expose benchmark JSON.
- Unified benchmark extraction across live logs, `bench-report.json`, iOS marker logs, and Android `BENCH_JSON` logs so per-function CI summaries are written with populated benchmark data.
- Fixed merged CI output generation to preserve every function under each target and emit a top-level `summary` for single-target runs.
- Fixed `cargo-mobench ci summarize` to read merged `{targets, ci}` outputs, recurse through nested target/function result directories, and fall back to raw `bench-report.json` when needed.

### v0.1.21

- Added a shared config-first project resolver across `build`, `run`, packaging, `list`, and `verify`.
- Added `--project-root` and `--crate-path` parity across the main CLI commands for custom repository layouts.
- `build --progress` now respects `mobench.toml` instead of assuming `bench-mobile`.
- Dotenv loading now follows the resolved project root and config path.
- `list` now discovers benchmarks from configured external crates instead of only legacy sample layouts.
- `verify --smoke-test` now reports external-crate smoke tests as unsupported instead of failing with an empty benchmark list.

### v0.1.14

- Added CI contract-oriented commands and workflows:
  - `cargo mobench ci run`
  - `cargo mobench config validate`
  - `cargo mobench devices resolve`
  - `cargo mobench fixture init|build|verify|cache-key`
  - `cargo mobench report summarize|github`
- Standardized CI outputs under `target/mobench/ci/` with schema-backed metadata.
- Added baseline comparison source support (`path|url|artifact:<path>`) and regression labels.
- Improved local action safety for workflow input handling and sticky PR comment publishing.
- Fixed iOS CI target setup (`x86_64-apple-ios`) and preserved CI outputs on regression exit.

### v0.1.13

- **Setup and teardown support**: `#[benchmark]` macro now supports `setup`, `teardown`, and `per_iteration` attributes for excluding expensive initialization from timing measurements
  ```rust
  fn setup_data() -> Vec<u8> { vec![0u8; 10_000_000] }

  #[benchmark(setup = setup_data)]
  fn process_data(data: &Vec<u8>) {
      // Only this is measured, not the setup
  }
  ```
- **New `check` command**: Validates prerequisites (NDK, Xcode, Rust targets, etc.) before building
  ```bash
  cargo mobench check --target android
  cargo mobench check --target ios
  ```
- **New `verify` command**: Validates registry, spec, and artifacts
- **New `summary` command**: Displays benchmark result statistics (avg/min/max/median)
- **New `devices` command**: Lists available BrowserStack devices with validation
- **`--progress` flag**: Simplified step-by-step output for `build` and `run` commands
- **Consolidated `mobench-runner` into `mobench-sdk`**: The timing harness is now part of `mobench-sdk` as the `timing` module, simplifying the dependency graph
- **SDK improvements**:
  - `#[benchmark]` macro now validates function signature at compile time (no params, returns `()`)
  - New `debug_benchmarks!()` macro for verifying benchmark registration
  - Better error messages with available benchmarks list
- **BrowserStack improvements**:
  - Better credential error messages with setup instructions
  - Artifact pre-flight validation before uploads
  - Upload progress indication with file sizes
  - Dashboard link printed immediately when build starts
  - Improved device fuzzy matching with suggestions
- **Fix iOS XCUITest test name mismatch**: Changed BrowserStack `only-testing` filter to use `testLaunchAndCaptureBenchmarkReport`

### v0.1.12

- **Fix iOS XCUITest BrowserStack detection**: Added Info.plist to the UITests target template, resolving issues where BrowserStack could not properly detect and run XCUITest bundles
- **Improved video capture for BrowserStack**: Increased post-benchmark delay from 0.5s to 5.0s to ensure benchmark results are captured in BrowserStack video recordings
- **Better UX during benchmark runs**: iOS app now shows "Running benchmarks..." text before results appear, providing visual feedback during execution
- **Template sync**: Synchronized top-level iOS/Android templates with SDK-embedded templates for consistency

### v0.1.11

- Initial public release with `--release` flag support
- `package-xcuitest` command for iOS BrowserStack testing
- Updated mobile timing display and documentation

MIT licensed — World Foundation 2026.
