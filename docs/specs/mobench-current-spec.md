# mobench Current Behavior And API Spec

Status: current source-of-truth product/API specification.

Release line: `0.1.43`.

Last updated: 2026-07-15.

This spec describes the behavior, CLI surface, configuration files, output
contracts, generated runner backends, and Rust APIs currently provided by
mobench. Historical design proposals are intentionally excluded.

## Product Scope

mobench is a Rust mobile benchmarking toolkit with three published crates:

- `mobench`: CLI orchestration and programmatic CLI API.
- `mobench-sdk`: timing harness, benchmark registry, generated runner support,
  Android/iOS builders, profiling helpers, UniFFI compatibility, and native C
  ABI support.
- `mobench-macros`: `#[benchmark]` proc macro.

mobench supports two product surfaces:

- Benchmark execution: build mobile artifacts, run benchmarks locally,
  host-only, or on BrowserStack, and write JSON/Markdown/CSV/plot outputs.
- Local native profiling: run native capture plans, produce normalized profile
  manifests, flamegraph artifacts, semantic phase summaries, and profile diffs.

## Workspace And Packages

The Cargo workspace contains:

- `crates/mobench`: CLI library and binaries `mobench` and `cargo-mobench`.
- `crates/mobench-sdk`: SDK library.
- `crates/mobench-macros`: proc macro library.
- `crates/sample-fns`: repository demo benchmarks.
- `examples/basic-benchmark`: minimal SDK integration.
- `examples/ffi-benchmark`: full generated FFI example.

Workspace package defaults:

- Edition: Rust 2024.
- MSRV: Rust 1.85.
- License: MIT.
- Current version: `0.1.43`.

## Benchmark Authoring

### `#[benchmark]`

The `#[benchmark]` macro registers benchmark functions at compile time through
`inventory`. Registered functions are discoverable by the SDK runtime and CLI.

Simple benchmark rules:

- Function takes no parameters.
- Function returns `()`.
- Function should be `pub` when linked into generated mobile runners.
- Benchmark work should pass values to `std::hint::black_box` or
  `mobench_sdk::black_box` when needed to prevent optimization.

Example:

```rust
use mobench_sdk::benchmark;

#[benchmark]
pub fn fibonacci_30() {
    let result = fibonacci(30);
    std::hint::black_box(result);
}
```

### Setup

`#[benchmark(setup = setup_fn)]` runs setup once before measured iterations and
passes the setup value by reference to the benchmark.

Rules:

- Setup returns the input type.
- Benchmark accepts one parameter by reference.
- Setup time is excluded from measured samples.

```rust
fn setup_data() -> Vec<u8> {
    vec![42; 1024 * 1024]
}

#[benchmark(setup = setup_data)]
pub fn checksum(data: &Vec<u8>) {
    let sum: u64 = data.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

### Per-Iteration Setup

`#[benchmark(setup = setup_fn, per_iteration)]` runs setup before each measured
iteration and passes the setup value by value.

Use this when the benchmark mutates or consumes its input.

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

### Teardown

`#[benchmark(setup = setup_fn, teardown = teardown_fn)]` runs teardown after
measured execution and passes the setup value to teardown.

Teardown is intended for resources such as files, connections, or handles that
must be cleaned up outside measured timing.

### Macro Validation

The macro validates unsupported signatures at compile time. Invalid examples
include async functions, simple benchmarks with parameters, and benchmark
functions that return non-unit values.

## SDK Runtime API

### Core Types

`mobench_sdk::timing::BenchSpec`

- `name: String`: benchmark function name, often fully-qualified.
- `iterations: u32`: measured iterations. Must be greater than zero.
- `warmup: u32`: warmup iterations. May be zero.
- `BenchSpec::new(name, iterations, warmup)` validates iterations.

`mobench_sdk::timing::BenchSample`

- `duration_ns: u64`: wall-clock measured iteration duration.
- `cpu_time_ms: Option<f64>`: per-iteration CPU time when available.
- `peak_memory_kb: Option<u64>`: baseline-adjusted measured peak memory growth.
- `process_peak_memory_kb: Option<u64>`: measured process peak memory.

`mobench_sdk::timing::BenchReport`

- `spec: BenchSpec`
- `samples: Vec<BenchSample>`
- `phases: Vec<SemanticPhase>`
- `timeline: Vec<HarnessTimelineSpan>`
- Statistics helpers include `mean_ns`, `median_ns`, `std_dev_ns`, `min_ns`,
  `max_ns`, and percentile helpers.

`mobench_sdk::types::Target`

- `Android`
- `Ios`
- `Both`
- `as_str()` returns `android`, `ios`, or `both`.

`mobench_sdk::types::FfiBackend`

- `Uniffi`, serialized as `uniffi`.
- `NativeCAbi`, serialized as `native-c-abi`.
- `uses_uniffi()` is true only for the UniFFI backend.

### Runtime Functions

`mobench_sdk::run_benchmark(spec)`

- Looks up the benchmark by name in the inventory registry.
- Executes warmup and measured iterations.
- Returns `RunnerReport` on success.
- Returns `BenchError::UnknownFunction` when no registered function matches.

`mobench_sdk::BenchmarkBuilder`

- Builder for programmatic benchmark execution.
- Defaults: `iterations = 100`, `warmup = 10`.
- Methods: `new(function)`, `iterations(n)`, `warmup(n)`, `run()`.

`mobench_sdk::discover_benchmarks`, `find_benchmark`,
`list_benchmark_names`

- Registry discovery helpers available with the `registry`/`full` features.

`mobench_sdk::debug_benchmarks!()`

- Generates a local debug printer for registered benchmarks.

### Timing Helpers

`mobench_sdk::timing::run_closure(spec, closure)`

- Runs a closure with warmup and measured iterations.
- Returns `BenchReport`.
- Used by generated macro registration.

`mobench_sdk::timing::profile_phase(name, closure)`

- Measures named semantic phases inside a benchmark.
- Local profile summaries render semantic phase totals separately from native
  flamegraph data.

### SDK Feature Flags

- `default = ["full"]`
- `full = ["registry", "builders", "codegen"]`
- `registry = ["dep:mobench-macros", "dep:inventory"]`
- `builders = ["codegen", "dep:toml"]`
- `codegen = ["dep:include_dir", "dep:toml"]`
- `runner-only = []`

## Native C ABI

The native C ABI allows generated Android/iOS runners to call the benchmark
crate directly without UniFFI-generated Kotlin/Swift bindings in the measured
path.

Benchmark crates opt in through:

```rust
mobench_sdk::export_native_c_abi!();
```

Exported symbols:

- `mobench_run_benchmark_json(spec_ptr, spec_len, out) -> i32`
- `mobench_free_buf(buf)`
- `mobench_last_error_message() -> *const c_char`

`MobenchBuf`

- `ptr: *mut u8`
- `len: usize`
- `cap: usize`

Return codes:

- `0`: success.
- `1`: recoverable error. Read `mobench_last_error_message`.
- `2`: panic caught across the native C ABI boundary.

Safety contract:

- `spec_ptr` must point to `spec_len` initialized bytes when `spec_len > 0`.
- `out` must be a valid writable `MobenchBuf`.
- The output buffer is owned by Rust and must be returned exactly once through
  `mobench_free_buf`.
- Error message pointers are thread-local and valid until the next ABI call on
  the same thread.

Payload contract:

- Input is JSON-serialized `BenchSpec`.
- Output is JSON-serialized `RunnerReport`.

## Generated Runner Backends

`[project].ffi_backend` in `mobench.toml` selects generated mobile runner
behavior.

`uniffi`

- Default compatibility backend.
- Generates Kotlin/Swift bindings from Rust/UniFFI types.
- Generated mobile runners call the UniFFI exposed benchmark entrypoint.

`native-c-abi`

- Direct mobench JSON C ABI backend.
- Skips UniFFI binding generation for benchmark invocation.
- Generated mobile runners call `mobench_run_benchmark_json`.
- Intended for engine benchmarks where binding overhead would distort timing or
  memory.

## Project Configuration

`mobench.toml` controls project/build settings.

```toml
[project]
crate = "my-bench-crate"
library_name = "my_bench_crate"
output_dir = "target/mobench"
ffi_backend = "uniffi"

[android]
package = "com.example.bench"
min_sdk = 24
target_sdk = 34
abis = ["arm64-v8a"]

[ios]
bundle_id = "com.example.bench"
deployment_target = "15.0"

[benchmarks]
default_function = "my_crate::my_benchmark"
default_iterations = 100
default_warmup = 10
```

`project.output_dir` must be a relative descendant of the project root.
Absolute paths, parent traversal, and pre-existing symlink components are
configuration errors detected before generation or cleanup begins.

Configuration resolution precedence:

1. Explicit `--project-root` and `--crate-path`.
2. Explicit `--config`.
3. Discovered `mobench.toml`.
4. Cargo workspace metadata.
5. Git root.
6. Legacy `bench-mobile/` fallback.

CLI flags override config file values.

## Run Configuration

`bench-config.toml` controls benchmark run settings.

```toml
target = "android"
function = "my_crate::my_benchmark"
iterations = 100
warmup = 10
device_matrix = "device-matrix.yaml"
device_tags = ["default"]

[browserstack]
app_automate_username = "${BROWSERSTACK_USERNAME}"
app_automate_access_key = "${BROWSERSTACK_ACCESS_KEY}"
project = "my-project-benchmarks"

[ios_xcuitest]
app = "target/mobench/ios/BenchRunner.ipa"
test_suite = "target/mobench/ios/BenchRunnerUITests.zip"
```

`--device-matrix <FILE>` overrides `device_matrix` from `--config`.

## Device Matrix

Device matrices select BrowserStack devices deterministically.

```yaml
devices:
  - name: "Google Pixel 7-13.0"
    os: "android"
    os_version: "13.0"
    tags: ["default", "pixel"]
  - name: "iPhone 14-16"
    os: "ios"
    os_version: "16"
    tags: ["default", "iphone"]
```

Device selection sources:

- Explicit `--devices`.
- `--device-matrix` plus `--device-tags`.
- Built-in device profiles for profile resolution.
- Config-provided matrix values.

## CLI Surface

Global flags:

- `--dry-run`
- `--verbose` / `-v`
- `--yes`
- `--non-interactive`
- `--help`
- `--version`

Commands:

- `run`: run benchmarks locally, host-only, or on BrowserStack.
- `init`: scaffold a base config file.
- `plan`: generate a sample device matrix file.
- `config validate`: validate run configuration and referenced files.
- `doctor`: validate local and CI prerequisites/configuration.
- `ci run`: run full CI benchmark flow.
- `ci merge-split-runs`: merge one-sample CI summaries into standard CI outputs.
- `fetch`: fetch BrowserStack build artifacts.
- `compare`: compare two run summaries for regressions.
- `init-sdk`: initialize a benchmark project from SDK templates.
- `build`: build mobile artifacts.
- `package-ipa`: package iOS app as IPA.
- `package-xcuitest`: package XCUITest runner.
- `list`: list discovered benchmark functions.
- `verify`: verify registry, spec, artifacts, and optional smoke test.
- `summary`: display statistics for a benchmark result JSON file.
- `devices`: list and validate BrowserStack devices.
- `devices resolve`: resolve device matrix/profile selections.
- `fixture`: fixture lifecycle helpers.
- `report summarize`: generate Markdown summary from standardized JSON output.
- `report github`: generate or publish sticky GitHub PR comments.
- `profile run`: run local/native profile capture planning/execution.
- `profile summarize`: render profile summaries.
- `profile diff`: compare two profile sessions.
- `check`: check platform build prerequisites.

## `run` Behavior

`mobench run` is the single-command benchmark flow.

Behavior:

1. Resolve project, crate, target, function, iterations, warmup, devices, and
   backend.
2. For normal device runs, build Rust libraries and generated mobile apps.
3. For BrowserStack runs, package app artifacts and required test runners.
4. Upload artifacts and schedule BrowserStack automation when devices are
   requested.
5. Fetch and normalize results when provider returns them.
6. Write JSON output and optional Markdown/CSV outputs.

Key options:

- `--target <android|ios>`
- `--function <FUNCTION>`
- `--project-root <PATH>`
- `--crate-path <PATH>`
- `--iterations <N>`
- `--warmup <N>`
- `--devices <DEVICES>`
- `--device-matrix <FILE>`
- `--device-tags <TAGS>`
- `--config <FILE>`
- `--output <FILE>`
- `--summary-csv`
- `--ci`
- `--baseline <path|url|artifact:path>`
- `--regression-threshold-pct <N>`
- `--junit <FILE>`
- `--local-only`
- `--release`
- `--ios-app <FILE>`
- `--ios-test-suite <FILE>`
- `--fetch`
- `--fetch-output-dir <DIR>`
- `--fetch-poll-interval-secs <N>`
- `--fetch-timeout-secs <N>`
- `--progress`

`--local-only` skips mobile builds and runs the host harness.

BrowserStack collection waits at most 900 seconds by default. The bound covers
observed provider queue latency while remaining overrideable through
`--fetch-timeout-secs`.

`--release` is recommended for BrowserStack to reduce upload size.

For iOS BrowserStack runs, IPA and XCUITest packages are created automatically
unless both `--ios-app` and `--ios-test-suite` are provided.

## `ci run` Behavior

`mobench ci run` writes stable CI contract outputs.

Key options:

- `--target <android|ios|both>`
- `--function <FUNCTION>`
- `--functions <FUNCTIONS>`
- `--iterations <N>` default `100`
- `--warmup <N>` default `10`
- `--devices`, `--device-matrix`, `--device-tags`
- `--config`
- `--baseline`
- `--regression-threshold-pct` default `5`
- `--junit`
- `--local-only`
- `--release`
- `--ios-app`, `--ios-test-suite`
- `--fetch`, fetch timeout/poll options
- `--progress`
- `--output-dir` default `target/mobench/ci`
- `--requested-by`
- `--pr-number`
- `--request-command`
- `--mobench-ref`
- `--plots <auto|off|require>`

Outputs:

- `summary.json`
- `summary.md`
- `results.csv`
- `plots/*.svg` when plot rendering is enabled and available.

`--functions` accepts comma-separated values or a JSON array passed as one
argument. `--function` is single-function sugar.

Regression exit semantics:

- Normal success exits `0`.
- Regression threshold failures are represented as regression failures and by
  the programmatic API as `regression_detected = true`.

## `ci merge-split-runs` Behavior

`mobench ci merge-split-runs` merges CI outputs from lanes that run one measured
sample per invocation. Inputs live under `--samples-dir` as
`sample-*/summary.json`.

Each input summary must contain exactly one device and one benchmark result. The
benchmark must match `--function`, every sample must be for `--device`, and the
merged measured sample count must equal `--iterations`. Warmup count is retained
in Markdown/report output but is not treated as a measured sample.

Outputs in `--output-dir`:

- `summary.json`
- `summary.md`
- `results.csv`

The command combines benchmark samples, derives `samples_ns`, recomputes
`min_ns`, `max_ns`, `mean_ns`, `median_ns`, `p95_ns`, and emits resource metric
columns using the same summary schema as normal `ci run` output. Existing
report, plot, PR comment, and comparison tooling can consume merged outputs
unchanged.

## Reporting Outputs

Summary JSON contains normalized benchmark/device/function rows and metadata.

Markdown summaries include:

- Device/function timing table.
- Wall-clock statistics.
- CPU columns when available.
- Peak memory columns when available.
- Device comparison plots when enabled.

CSV rows include benchmark-scoped resource columns:

- `cpu_total_ms`
- `cpu_median_ms`
- `peak_memory_kb`

Missing resource metrics are emitted as blank CSV fields.

## BrowserStack Behavior

Credentials resolve in this order:

1. Config file values with environment expansion.
2. Environment variables:
   - `BROWSERSTACK_USERNAME`
   - `BROWSERSTACK_ACCESS_KEY`
   - `BROWSERSTACK_PROJECT`
3. `.env.local`.

Android BrowserStack runs upload:

- App APK.
- Espresso/androidTest APK.

iOS BrowserStack runs upload:

- App IPA or zipped app bundle.
- XCUITest runner package.

Device commands:

- `mobench devices`
- `mobench devices --platform android`
- `mobench devices --platform ios`
- `mobench devices --json`
- `mobench devices --validate "Google Pixel 7-13.0"`
- `mobench devices resolve --platform android --profile default`

Invalid device specs should produce suggestions when similar devices are known.

## Build Outputs

Default output root: `target/mobench/`.

Android output:

- Generated Android project under `target/mobench/android/`.
- APK under Gradle output directories.
- Native libraries under generated `jniLibs/{abi}` directories.

iOS output:

- Generated iOS project under `target/mobench/ios/BenchRunner/`.
- `<library_name>.xcframework`.
- Optional `BenchRunner.ipa`.
- Optional `BenchRunnerUITests.zip`.

iOS xcframework slices are manually constructed. Framework binary names,
headers, module maps, bundle IDs, and platform identifiers must match Xcode
expectations. The build signs the xcframework with ad-hoc signing when needed.

## Mobile Spec Injection

The CLI writes benchmark parameters into generated app artifacts.

Runtime spec fields:

- Function name.
- Iterations.
- Warmup.

Android runners read:

- Intent extras when provided.
- `bench_spec.json` asset fallback.

iOS runners read:

- Environment variables.
- Launch arguments.
- `bench_spec.json` bundle resource fallback.

Generated runners emit benchmark JSON markers that the CLI can parse from local
or BrowserStack logs.

## Profiling

`mobench profile run` plans or executes native profiling sessions.

Key options:

- `--target <android|ios>`
- `--function <FUNCTION>`
- `--crate-path <PATH>`
- `--config <FILE>`
- `--output-dir <DIR>` default `target/mobench/profile`
- `--trace-events-output <FILE>`
- `--device <DEVICE>`
- `--os-version <VERSION>`
- `--profile <PROFILE>`
- `--device-matrix <FILE>`
- `--provider <local|browserstack>` default `local`
- `--backend <auto|android-native|ios-instruments|rust-tracing>` default `auto`
- `--format <native|processed|both>` default `both`
- `--warmup-mode <cold|warm>`

Capability matrix:

| Provider | Backend | Behavior |
| --- | --- | --- |
| `local` | `android-native` | Attempts `simpleperf` capture and symbolization. |
| `local` | `ios-instruments` | Attempts simulator-host `sample` capture and flamegraph generation. |
| `local` | `rust-tracing` | Planned manifest/trace contract only. |
| `browserstack` | `android-native` | Unsupported for native capture. |
| `browserstack` | `ios-instruments` | Unsupported for native capture. |
| `browserstack` | `rust-tracing` | Unsupported. |

Profile manifest sections:

- `native_capture`: native stack artifacts, symbolization, viewer hints.
- `semantic_profile`: benchmark phase data from `profile_phase`.
- `capture_metadata`: device resolution, settings, warnings.

Session outputs:

- `<output-dir>/<run-id>/profile.json`
- `<output-dir>/<run-id>/summary.md`
- `<output-dir>/<run-id>/artifacts/processed/stacks.folded`
- `<output-dir>/<run-id>/artifacts/processed/native-report.txt`
- `<output-dir>/<run-id>/artifacts/processed/flamegraph.html`
- `<output-dir>/<run-id>/artifacts/semantic/phases.json` when semantic phases
  exist.

Convenience copies:

- `<output-dir>/profile.json`
- `<output-dir>/summary.md`

`profile diff` compares two profile manifests and writes a diff bundle under
`target/mobench/profile/diff/` by default.

## Programmatic `mobench` API

The `mobench` crate exposes programmatic helpers for CI integrations.

`DeviceSelection`

- `devices: Vec<String>`
- `device_matrix: Option<PathBuf>`
- `device_tags: Vec<String>`

`RunRequest`

- `target`
- `function`
- `crate_path`
- `iterations`
- `warmup`
- `device_selection`
- `config`
- `baseline`
- `regression_threshold_pct`
- `junit`
- `local_only`
- `release`
- `ios_app`
- `ios_test_suite`
- `ios_completion_timeout_secs`
- `fetch`
- `fetch_output_dir`
- `fetch_poll_interval_secs`
- `fetch_timeout_secs`
- `progress`
- `output_dir`
- `plots`

`Report`

- `summary_json`
- `summary_md`
- `results_csv`

`RunResult`

- `target`
- `report`
- `exit_code`
- `regression_detected`

`run_request(request)`

- Invokes the current `mobench` executable.
- Normalizes CI output names in `request.output_dir`.
- Writes `summary.json`, `summary.md`, and `results.csv`.
- Returns `RunResult`.

Render helpers:

- `render_summary_markdown_from_json`
- `render_compare_markdown_from_json`
- `render_profile_markdown_from_json`

## Verification And Diagnostics

`mobench check`

- Validates platform build prerequisites.
- Android checks include NDK, `cargo-ndk`, and Rust targets.
- iOS checks include Xcode, XcodeGen, and Rust targets.

`mobench doctor`

- Validates local and CI prerequisites/configuration.
- Includes BrowserStack credential checks unless disabled.

`mobench config validate`

- Validates run configuration and referenced matrix/settings.

`mobench verify`

- Validates benchmark setup, registry, spec, artifacts, and optional smoke test.
- `--smoke-test` is only supported for benchmark crates linked into the CLI
  binary.
- External crates should use `list` and `verify --check-artifacts`.

`mobench list`

- Lists discovered benchmark functions from the resolved benchmark crate.

## Schemas

Machine-readable schemas live under `docs/schemas/`:

- `ci-contract-v1.schema.json`
- `summary-v1.schema.json`
- `trace-events-v1.schema.json`

These schemas are part of the CI/reporting compatibility contract.

## Compatibility Boundaries

Compatibility-sensitive surfaces:

- `#[benchmark]` syntax and validation.
- `mobench-sdk` top-level re-exports.
- `FfiBackend` serialized values.
- Native C ABI exported symbol names and buffer ownership contract.
- CLI flags used in CI.
- JSON, Markdown, CSV, profile, and trace-event output contracts.
- Generated template input/output paths.

Changes that rename public items, change serialized field names/units, change
default feature composition, or alter output paths require release-note
migration guidance.
