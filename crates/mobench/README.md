# mobench

Command-line tool for building, running, reporting, and profiling Rust mobile
benchmarks on Android and iOS.

Current release: **0.1.46**.

## Install

```bash
cargo binstall mobench
```

Or build with Cargo:

```bash
cargo install mobench
```

The package installs both direct and Cargo-subcommand entry points:

```bash
mobench --help
cargo mobench --help
```

## Quick Start

```bash
# Scaffold a config
cargo mobench init --target android --output bench-config.toml

# Validate local prerequisites and config
cargo mobench doctor --target both
cargo mobench config validate --config bench-config.toml
cargo mobench check --target android
cargo mobench check --target ios

# Build mobile artifacts
cargo mobench build --target android --progress
cargo mobench build --target ios --progress

# List and verify discovered benchmarks
cargo mobench list
cargo mobench verify --target android --check-artifacts

# Host-only smoke run
cargo mobench run --target android --function my_crate::my_benchmark --local-only

# BrowserStack run
cargo mobench run \
  --target android \
  --function my_crate::my_benchmark \
  --devices "Google Pixel 7-13.0" \
  --release
```

Always use `--release` for BrowserStack runs unless you are intentionally
debugging large development artifacts.

## Core Commands

| Command | Purpose |
| --- | --- |
| `init` | Scaffold a base config file. |
| `init-sdk` | Initialize a benchmark project from SDK templates. |
| `doctor` | Validate local and CI prerequisites/configuration. |
| `config validate` | Validate config files and referenced matrix/settings. |
| `check` | Check Android or iOS build prerequisites. |
| `build` | Build Android/iOS mobile artifacts. |
| `run` | Run benchmarks locally, host-only, or on BrowserStack. |
| `ci run` | Run the full CI benchmark flow and write stable contract outputs. |
| `ci merge-split-runs` | Merge one-sample CI summaries into standard CI outputs. |
| `devices` | List BrowserStack devices. |
| `devices resolve` | Resolve deterministic device sets from matrix/profile. |
| `fixture` | Manage reproducible CI fixtures. |
| `report` | Render summaries and GitHub PR comments/checks. |
| `fetch` | Fetch BrowserStack build artifacts. |
| `summary` | Display statistics for a benchmark result JSON file. |
| `profile` | Run, summarize, and diff local native profile captures. |
| `package-ipa` | Package iOS app as IPA. |
| `package-xcuitest` | Package XCUITest runner for BrowserStack. |

## Configuration

`mobench.toml` controls project-level build settings:

```toml
[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"
output_dir = "target/mobench"
ffi_backend = "uniffi" # default; also supports "native-c-abi" and "boltffi"

# FFI backend for generated mobile runners: "uniffi" (default), "native-c-abi", or "boltffi".
# Use "boltffi" for BoltFFI-generated bindings, or "native-c-abi" for direct C ABI calls.
# ffi_backend = "uniffi"

[android]
package = "com.example.bench"
min_sdk = 24
target_sdk = 34

[ios]
bundle_id = "com.example.bench"
deployment_target = "15.0"

[benchmarks]
default_function = "my_crate::my_benchmark"
default_iterations = 100
default_warmup = 10
```

`ffi_backend = "native-c-abi"` selects generated runners that call mobench's
direct JSON C ABI. Benchmark crates using that backend should export:

```rust
mobench_sdk::export_native_c_abi!();
```

`ffi_backend = "boltffi"` selects generated runners that call BoltFFI-generated
Kotlin/Swift bindings. Benchmark crates using that backend should export a JSON
entrypoint with `#[boltffi::export]`.

`bench-config.toml` controls a run:

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

Resolution precedence is explicit CLI flags, explicit `--config`, discovered
`mobench.toml`, Cargo workspace metadata, git root, then legacy `bench-mobile/`.

## CI Contract

```bash
cargo mobench ci run \
  --target android \
  --function my_crate::my_benchmark \
  --devices "Google Pixel 7-13.0" \
  --release \
  --plots auto
```

Outputs default to `target/mobench/ci/`:

- `summary.json`
- `summary.md`
- `results.csv`
- `plots/*.svg` when plot rendering is enabled

Use report helpers to publish or re-render outputs:

```bash
cargo mobench report summarize \
  --summary target/mobench/ci/summary.json \
  --plots auto

cargo mobench report github \
--pr 123 \
--summary target/mobench/ci/summary.json
```

Long or fragile CI lanes can run one measured sample per BrowserStack
invocation, then merge `sample-*/summary.json` outputs back into the standard
CI contract:

```bash
cargo mobench ci merge-split-runs \
--samples-dir target/mobench/ci/android/my_crate__my_benchmark/device/split \
--output-dir target/mobench/ci/android/my_crate__my_benchmark/device \
--function my_crate::my_benchmark \
--device "Google Pixel 7-13.0" \
--iterations 5 \
--warmup 1
```

The merge command validates one device and one benchmark per input summary,
checks the requested function and device, requires exactly `--iterations`
measured samples, and writes `summary.json`, `summary.md`, and `results.csv` in
`--output-dir`.

## BrowserStack

Credentials are read from config, environment, or `.env.local`:

```bash
export BROWSERSTACK_USERNAME="your_username"
export BROWSERSTACK_ACCESS_KEY="your_access_key"
```

Useful device commands:

```bash
cargo mobench devices --platform android
cargo mobench devices --platform ios
cargo mobench devices --validate "Google Pixel 7-13.0"
cargo mobench devices resolve \
  --platform android \
  --profile default \
  --device-matrix device-matrix.yaml
```

## Local Native Profiling

```bash
cargo mobench profile run \
  --target android \
  --function my_crate::my_benchmark \
  --provider local \
  --backend android-native \
  --trace-events-output target/mobench/profile/trace-events.json

cargo mobench profile summarize --profile target/mobench/profile/profile.json
cargo mobench profile diff \
  --baseline target/mobench/profile/baseline/profile.json \
  --candidate target/mobench/profile/candidate/profile.json \
  --normalize
```

Supported local backends:

- `local + android-native`: attempts `simpleperf` capture, symbolization,
  folded stacks, native report, SVGs, and `flamegraph.html`.
- `local + ios-instruments`: attempts simulator-host `sample` capture, folded
  stacks, native report, SVGs, and `flamegraph.html`.
- `local + rust-tracing`: planned trace contract only.

BrowserStack native profiling is unsupported in this release; use normal
BrowserStack benchmark runs for timing, CPU, and memory metrics.

## Programmatic CLI API

The crate exposes typed integration helpers:

- `RunRequest`
- `RunResult`
- `DeviceSelection`
- `Report`
- `run_request`
- `render_summary_markdown_from_json`
- `render_profile_markdown_from_json`

## License

MIT licensed, World Foundation 2026.
