# mobench

Mobile benchmarking CLI for Rust.

The `mobench` CLI handles project setup, mobile artifact builds, benchmark execution, result reporting, and local native profiling for Android and iOS. Benchmark execution can run locally or on BrowserStack. Native profiling is currently local-first.

## Installation

```bash
cargo binstall mobench
```

Or build from source:

```bash
cargo install mobench
```

Use as a Cargo subcommand:

```bash
cargo install mobench
cargo mobench --help
```

## Quick Start

### 1. Initialize Your Project

```bash
# Create mobile benchmarking setup for Android
cargo mobench init --target android

# Or for iOS
cargo mobench init --target ios

# Or for both platforms
cargo mobench init --target both
```

This creates:
- `bench-mobile/` - FFI wrapper crate with UniFFI bindings
- `android/` or `ios/` - Platform-specific app projects (generated to output directory)
- `bench-config.toml` - Run configuration file
- `mobench.toml` - Project configuration file (when using `init`)
- `benches/example.rs` - Example benchmarks (with `--examples`)

Generated scaffolding still uses `bench-mobile/` by default, but existing repositories can point mobench at any benchmark crate through `mobench.toml`, `--project-root`, or `--crate-path`.

### 2. Write Benchmarks

```rust
// benches/my_benchmarks.rs
use mobench_sdk::benchmark;

#[benchmark]
fn fibonacci_30() {
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

### 3. Build for Mobile

```bash
# Build Android APK
cargo mobench build --target android

# Build iOS app
cargo mobench build --target ios
```

### 4. Run Benchmarks

Local device workflow (builds artifacts and writes the run spec; launch the app manually):
```bash
cargo mobench run --target android --function fibonacci_30 --iterations 50
```

On real devices via BrowserStack:
```bash
export BROWSERSTACK_USERNAME=your_username
export BROWSERSTACK_ACCESS_KEY=your_access_key

cargo mobench run \
  --target android \
  --function fibonacci_30 \
  --devices "Google Pixel 7-13.0" \
  --iterations 100 \
  --warmup 10 \
  --release
```

**Note**: Always use the `--release` flag for BrowserStack runs. Debug builds are significantly larger (~544MB vs ~133MB for release) and may cause upload timeouts.

### 5. Run a Local Profiling Session

```bash
# Capture a local Android native profile
cargo mobench profile run \
  --target android \
  --function fibonacci_30 \
  --provider local \
  --backend android-native

# Summarize the latest profile session
cargo mobench profile summarize \
  --profile target/mobench/profile/profile.json
```

Local profile runs now attempt real native capture for:

- `local + android-native`: `simpleperf`, symbolized folded stacks, `native-report.txt`, full/focused SVGs, and `flamegraph.html`
- `local + ios-instruments`: simulator-host `sample`, folded stacks, `native-report.txt`, full/focused SVGs, and `flamegraph.html`

Each run writes a normalized manifest under `target/mobench/profile/<run-id>/`
and refreshes `target/mobench/profile/profile.json` plus `summary.md` as
latest-run convenience copies. BrowserStack native profiling remains explicitly
unsupported in this release.

## Commands

### `init` - Initialize Project

Create mobile benchmarking infrastructure:

```bash
cargo mobench init [OPTIONS]
```

**Options:**
- `--target <android|ios|both>` - Target platform (default: android)
- `--output <FILE>` - Config file path (default: bench-config.toml)

**Example:**
```bash
cargo mobench init --target both --output my-bench.toml
```

### `build` - Build Mobile Apps

Cross-compile and package for mobile platforms:

```bash
cargo mobench build --target <android|ios> [OPTIONS]
```

**Options:**
- `--target <android|ios>` - Platform to build for (required)
- `--release` - Build in release mode (default: debug)
- `--output-dir <DIR>` - Output directory for mobile artifacts (default: `target/mobench/`)
- `--project-root <PATH>` - Project root containing `mobench.toml` or the Cargo workspace
- `--crate-path <PATH>` - Path to the benchmark crate (default: resolve from flags, `mobench.toml`, workspace, or git root)
- `--dry-run` - Print what would be done without making changes
- `--verbose` / `-v` - Print verbose output including all commands

**Examples:**
```bash
# Build Android APK in release mode
cargo mobench build --target android --release

# Build iOS xcframework
cargo mobench build --target ios

# Preview build without making changes
cargo mobench build --target android --dry-run

# Build with verbose output
cargo mobench build --target ios --verbose

# Build to custom output directory
cargo mobench build --target android --output-dir ./my-output
```

**Outputs:**
- Android: `target/mobench/android/app/build/outputs/apk/debug/app-debug.apk`
- iOS: `target/mobench/ios/<library_name>.xcframework` (derived from `[project].library_name` or the resolved crate name)

### `run` - Run Benchmarks

Execute benchmarks on devices:

```bash
cargo mobench run --target <android|ios> --function <NAME> [OPTIONS]
```

**Options:**
- `--target <android|ios>` - Platform (required)
- `--function <NAME>` - Benchmark function name (required)
- `--project-root <PATH>` - Project root containing `mobench.toml` or the Cargo workspace
- `--crate-path <PATH>` - Path to the benchmark crate directory containing `Cargo.toml`
- `--iterations <N>` - Number of iterations (default: 100)
- `--warmup <N>` - Warmup iterations (default: 10)
- `--devices <LIST>` - Comma-separated device list for BrowserStack
- `--device-matrix <FILE>` - Load devices from a matrix YAML file (overrides `device_matrix` from `--config` when both are provided)
- `--device-tags <TAG>` - Filter device matrix by tag (repeatable / comma-separated)
- `--local-only` - Skip mobile builds (no device run)
- `--config <FILE>` - Load run spec from config file
- `--ios-app <FILE>` - iOS .ipa or zipped .app for BrowserStack
- `--ios-test-suite <FILE>` - iOS XCUITest runner (.zip or .ipa)
- `--output <FILE>` - Save results to JSON file (default: target/mobench/results.json)
- `--summary-csv` - Write CSV summary alongside JSON/Markdown
- `--fetch` - Fetch BrowserStack results after completion
- `--ci` - CI mode (step summary + regression exit codes)
- `--baseline <path|url|artifact:<path>>` - Compare against baseline summary (non-zero on regressions); if baseline resolves to the output file path, mobench snapshots the previous file first
- `--regression-threshold-pct <N>` - Regression threshold percentage (default: 5.0)
- `--junit <FILE>` - Write JUnit XML report

**Outputs:**
- JSON summary (default: `target/mobench/results.json`)
- Markdown summary (same base name, `.md`)
- CSV summary (same base name, `.csv`, when `--summary-csv` is set)

**Examples:**
```bash
# Run locally (no BrowserStack devices specified)
cargo mobench run --target android --function fibonacci_30

# Run from a custom workspace layout
cargo mobench run \
  --project-root . \
  --crate-path ./crates/zk-mobile-bench \
  --target ios \
  --function zk_mobile_bench::bench_query_proof_generation \
  --dry-run

# Run on BrowserStack devices (use --release for smaller APK)
cargo mobench run \
  --target android \
  --function sha256_hash \
  --devices "Google Pixel 7-13.0,Samsung Galaxy S23-13.0" \
  --iterations 50 \
  --release \
  --output results.json

# Run on iOS with auto-fetch (use --release for smaller artifacts)
cargo mobench run \
  --target ios \
  --function json_parse \
  --devices "iPhone 14-16,iPhone 15-17" \
  --release \
  --fetch
```

### `ci run` - One-command CI Orchestration

Run build/package/run/fetch/report end-to-end with stable CI output files:

```bash
cargo mobench ci run --target <android|ios|both> --function <NAME> [OPTIONS]
```

**Contract outputs (default directory: `target/mobench/ci/`):**
- `summary.json`
- `summary.md`
- `results.csv`

`summary.json` includes a `ci` section with metadata fields:
- `requested_by`
- `pr_number`
- `request_command`
- `mobench_ref`
- `mobench_version`

Stable output references:
- `https://github.com/worldcoin/mobile-bench-rs/blob/dev/README.md` CI section
- `https://github.com/worldcoin/mobile-bench-rs/blob/dev/docs/schemas/summary-v1.schema.json`
- `https://github.com/worldcoin/mobile-bench-rs/blob/dev/docs/schemas/ci-contract-v1.schema.json`
- `https://github.com/worldcoin/mobile-bench-rs/blob/dev/RELEASE_NOTES.md`

**Example:**
```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch

# Combined android + ios contract output
cargo mobench ci run \
  --target both \
  --function sample_fns::fibonacci \
  --local-only
```

When `--baseline` is omitted for `ci run`, mobench automatically uses the previous successful summary snapshot in the target output directory when present.

### `config validate` - Validate Run Config Contract

Validate `bench-config.toml` and referenced matrix/settings with contract-aligned issue categories:

```bash
cargo mobench config validate --config bench-config.toml
cargo mobench config validate --config bench-config.toml --format json
```

### `devices resolve` - Deterministic Matrix Resolution

Resolve matrix devices for a platform/profile without custom scripts:

```bash
cargo mobench devices resolve \
  --platform android \
  --profile default \
  --device-matrix device-matrix.yaml

cargo mobench devices resolve \
  --platform ios \
  --config bench-config.toml \
  --format json
```

### `fixture` - Fixture Lifecycle Commands

Manage reproducible fixture setup for CI:

```bash
# Create starter fixture files
cargo mobench fixture init

# Build fixture artifacts
cargo mobench fixture build --target both --release

# Verify fixture config + matrix resolution
cargo mobench fixture verify --config bench-config.toml

# Generate deterministic cache key
cargo mobench fixture cache-key --config bench-config.toml --format json
```

### `package-ipa` - Package iOS IPA

Create a signed IPA for BrowserStack:

```bash
cargo mobench package-ipa [OPTIONS]
```

**Options:**
- `--scheme <NAME>` - Xcode scheme (default: BenchRunner)
- `--method <adhoc|development>` - Signing method (default: adhoc)
- `--project-root <PATH>` - Project root containing `mobench.toml` or the Cargo workspace
- `--crate-path <PATH>` - Path to the benchmark crate directory containing `Cargo.toml`

**Example:**
```bash
cargo mobench package-ipa --method adhoc
```

**Output:** `target/mobench/ios/BenchRunner.ipa`

### `package-xcuitest` - Package XCUITest Runner

Create the XCUITest runner package required for BrowserStack iOS testing:

```bash
cargo mobench package-xcuitest [OPTIONS]
```

**Options:**
- `--scheme <NAME>` - Xcode scheme for UI tests (default: BenchRunnerUITests)
- `--project-root <PATH>` - Project root containing `mobench.toml` or the Cargo workspace
- `--crate-path <PATH>` - Path to the benchmark crate directory containing `Cargo.toml`

**Example:**
```bash
cargo mobench package-xcuitest
```

**Output:** `target/mobench/ios/BenchRunnerUITests.zip`

This command builds the XCUITest target and packages it into the zip format that BrowserStack expects for iOS test automation.

### `plan` - Generate Device Matrix

Create a template device matrix file:

```bash
cargo mobench plan [--output <FILE>]
```

**Example:**
```bash
cargo mobench plan --output devices.yaml
```

**Output:** `device-matrix.yaml`

```yaml
devices:
  - name: Google Pixel 7-13.0
    os: android
    os_version: "13.0"
    tags: [default, pixel]
  - name: iPhone 14-16
    os: ios
    os_version: "16"
    tags: [default, iphone]
```

### `list` - List Benchmarks

Show benchmarks discovered via `#[benchmark]`:

```bash
cargo mobench list
cargo mobench list --project-root . --crate-path ./crates/zk-mobile-bench
```

`list` uses the same config-first resolver as `build` and `run`, so custom crate names from `mobench.toml` are discovered without a `bench-mobile/` directory.

### `verify` - Validate Benchmark Setup

Validate registry, spec files, build artifacts, and optional smoke tests:

```bash
cargo mobench verify --target android --check-artifacts --function zk_mobile_bench::bench_query_proof_generation
```

**Options:**
- `--project-root <PATH>` - Project root containing `mobench.toml` or the Cargo workspace
- `--crate-path <PATH>` - Path to the benchmark crate directory containing `Cargo.toml`
- `--check-artifacts` - Validate resolved build outputs
- `--smoke-test` - Run a local minimal-iteration smoke test when supported

`verify --smoke-test` only works for benchmark crates linked into the `mobench` CLI binary. For external crates resolved through `mobench.toml`, `--project-root`, or `--crate-path`, use `cargo mobench list` plus `cargo mobench verify --check-artifacts`.

### `fetch` - Fetch Results

Download BrowserStack build artifacts:

```bash
cargo mobench fetch --target <android|ios> --build-id <ID> [OPTIONS]
```

**Options:**
- `--target <android|ios>` - Platform (required)
- `--build-id <ID>` - BrowserStack build ID (required)
- `--output-dir <DIR>` - Download directory (default: target/browserstack)

**Example:**
```bash
cargo mobench fetch \
  --target android \
  --build-id abc123def456 \
  --output-dir ./results
```

### `compare` - Compare Summaries

Compare two JSON run summaries and emit a Markdown report:

```bash
cargo mobench compare \
  --baseline results-v1.json \
  --candidate results-v2.json \
  --output comparison.md
```

### `report summarize` - Render CI Summary Markdown

Generate natural-language markdown from standardized output JSON:

```bash
cargo mobench report summarize --summary target/mobench/ci/summary.json
cargo mobench report summarize --summary target/mobench/ci/summary.json --output report.md
```

### `report github` - Sticky PR Comment Payload/Publish

Create or update sticky PR comments from standardized outputs. When the
standardized summary includes resource usage, the PR comment also shows
`CPU total (ms)` and `Peak memory` columns next to the timing statistics.

```bash
# Print comment body
cargo mobench report github --pr 123 --summary target/mobench/ci/summary.json

# Publish/update comment (requires GITHUB_TOKEN + GITHUB_REPOSITORY)
cargo mobench report github \
  --pr 123 \
  --summary target/mobench/ci/summary.json \
  --publish \
  --marker "<!-- mobench-report -->"
```

## Configuration

### Project Configuration (`mobench.toml`)

mobench automatically loads `mobench.toml` from the current directory or any parent directory:

```toml
[project]
# Name of the benchmark crate
crate = "zk-mobile-bench"

# Rust library name (typically crate name with hyphens replaced by underscores)
library_name = "zk_mobile_bench"

# Output directory for build artifacts (default: target/mobench/)
# output_dir = "target/mobench"

[android]
# Android package name
package = "com.example.bench"

# Minimum Android SDK version (default: 24)
min_sdk = 24

# Target Android SDK version (default: 34)
target_sdk = 34

[ios]
# iOS bundle identifier
bundle_id = "com.example.bench"

# iOS deployment target version (default: 15.0)
deployment_target = "15.0"

# Development team ID for code signing (optional)
# team_id = "YOUR_TEAM_ID"

[benchmarks]
# Default benchmark function to run
default_function = "my_crate::my_benchmark"

# Default number of benchmark iterations
default_iterations = 100

# Default number of warmup iterations
default_warmup = 10
```

CLI flags always override config file values when provided.
Resolution precedence is: `--project-root` / `--crate-path` → explicit `--config` → discovered `mobench.toml` → Cargo workspace root → git root → legacy `bench-mobile` fallback.

### Run Config File Format (`bench-config.toml`)

For BrowserStack runs, you can also use a separate run configuration:

```toml
target = "android"
function = "sample_fns::fibonacci"
iterations = 100
warmup = 10
device_matrix = "device-matrix.yaml"
device_tags = ["default"] # optional; filter devices by tag

[browserstack]
app_automate_username = "${BROWSERSTACK_USERNAME}"
app_automate_access_key = "${BROWSERSTACK_ACCESS_KEY}"
project = "my-project-benchmarks"

[ios_xcuitest]
app = "target/mobench/ios/BenchRunner.ipa"
test_suite = "target/mobench/ios/BenchRunnerUITests.zip"
```

### Device Matrix Format (`device-matrix.yaml`)

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

### Environment Variables

BrowserStack credentials can be provided via:

1. **Environment variables** (recommended):
   ```bash
   export BROWSERSTACK_USERNAME=your_username
   export BROWSERSTACK_ACCESS_KEY=your_access_key
   ```

2. **`.env.local` file**:
   ```
   BROWSERSTACK_USERNAME=your_username
   BROWSERSTACK_ACCESS_KEY=your_access_key
   ```

3. **Config file** with variable expansion:
   ```toml
   [browserstack]
   username = "${BROWSERSTACK_USERNAME}"
   access_key = "${BROWSERSTACK_ACCESS_KEY}"
   ```

## Requirements

### For Android

- **Android NDK** - Set `ANDROID_NDK_HOME` environment variable
- **cargo-ndk** - Install with `cargo install cargo-ndk`
- **Android SDK** - API level 24+ required
- **Gradle** - For building APKs (bundled with Android project)

### For iOS

- **macOS** with Xcode installed
- **Xcode Command Line Tools** - `xcode-select --install`
- **Rust iOS targets**:
  ```bash
  rustup target add aarch64-apple-ios
  rustup target add aarch64-apple-ios-sim
  rustup target add x86_64-apple-ios
  ```
- **XcodeGen** - Install with `brew install xcodegen`

### For BrowserStack

- **BrowserStack App Automate account** - [Sign up](https://www.browserstack.com/app-automate)
- **Credentials** - Username and access key from account settings

## Examples

### Benchmark Crypto Operations

```bash
# Initialize
cargo mobench init --target android

# Add benchmark
cat > benches/crypto.rs <<'EOF'
use mobench_sdk::benchmark;
use sha2::{Sha256, Digest};

#[benchmark]
fn sha256_1kb() {
    let data = vec![0u8; 1024];
    let hash = Sha256::digest(&data);
    std::hint::black_box(hash);
}
EOF

# Build
cargo mobench build --target android --release

# Run on multiple devices (use --release for BrowserStack)
cargo mobench run \
  --target android \
  --function sha256_1kb \
  --devices "Google Pixel 7-13.0,Samsung Galaxy S23-13.0,OnePlus 11-13.0" \
  --iterations 200 \
  --release \
  --output crypto-results.json
```

### Compare iOS Performance

```bash
# Run same benchmark on different iOS versions (use --release for BrowserStack)
cargo mobench run \
  --target ios \
  --function json_parse \
  --devices "iPhone 13-15,iPhone 14-16,iPhone 15-17" \
  --iterations 100 \
  --release \
  --fetch \
  --output ios-comparison.json
```

### CI Integration

Generate a ready-to-edit workflow + action wrapper:

```bash
cargo mobench ci init
```

This writes `.github/workflows/mobile-bench.yml` plus a local action in
`.github/actions/mobench/` that handles caching, Android setup, and artifact upload.

Example workflow excerpt:

```yaml
- uses: ./.github/actions/mobench
  with:
    command: cargo mobench ci run
    run-args: |
      --target android
      --function my_benchmark
      --devices "Google Pixel 7-13.0"
      --iterations 50
      --release
      --fetch
    ci: false
  env:
    BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
    BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
```

The local action currently supports `command` values `cargo mobench ci run` and `cargo mobench run`.

For CI dashboards, add `--junit path/to/results.junit.xml`.

### Typed Rust API

`mobench` also exposes a typed request/result surface for integrations:

```rust
use mobench::{DeviceSelection, MobileTarget, RunRequest, run_request};
use std::path::PathBuf;

let result = run_request(&RunRequest {
    target: MobileTarget::Android,
    function: "sample_fns::fibonacci".to_string(),
    iterations: 20,
    warmup: 5,
    device_selection: DeviceSelection {
        devices: vec!["Google Pixel 7-13.0".to_string()],
        device_matrix: None,
        device_tags: vec![],
    },
    config: None,
    baseline: None,
    regression_threshold_pct: 5.0,
    junit: None,
    local_only: true,
    release: false,
    ios_app: None,
    ios_test_suite: None,
    fetch: false,
    fetch_output_dir: PathBuf::from("target/browserstack"),
    fetch_poll_interval_secs: 5,
    fetch_timeout_secs: 300,
    progress: false,
    output_dir: PathBuf::from("target/mobench/ci"),
})?;
println!("summary: {}", result.report.summary_json.display());
```

## Workflow

```
┌─────────────────────┐
│ 1. cargo mobench    │
│    init             │
└──────────┬──────────┘
           │
           ↓
┌─────────────────────┐
│ 2. Write benchmarks │
│    with #[benchmark]│
└──────────┬──────────┘
           │
           ↓
┌─────────────────────┐
│ 3. cargo mobench    │
│    build            │
└──────────┬──────────┘
           │
           ↓
┌─────────────────────┐
│ 4. cargo mobench    │
│    run              │
└──────────┬──────────┘
           │
      ┌────┴────┐
      ↓         ↓
┌──────────┐ ┌──────────────┐
│  Local   │ │ BrowserStack │
│ Emulator │ │ Real Devices │
└──────────┘ └──────────────┘
```

## Troubleshooting

### Android NDK not found

```bash
export ANDROID_NDK_HOME=/path/to/ndk
```

Or install via Android Studio: Tools → SDK Manager → SDK Tools → NDK

### iOS code signing issues

For BrowserStack testing, use ad-hoc signing:

```bash
cargo mobench package-ipa --method adhoc
```

### BrowserStack authentication failed

Verify credentials:

```bash
echo $BROWSERSTACK_USERNAME
echo $BROWSERSTACK_ACCESS_KEY
```

Or check `.env.local` file exists and contains valid credentials.

### Benchmark function not found

Ensure:
1. Function has `#[benchmark]` attribute
2. Function is compiled into the mobile binary
3. Function name matches exactly (case-sensitive)

## Part of mobench

This CLI is part of the mobench ecosystem:

- **[mobench](https://crates.io/crates/mobench)** - This crate (CLI tool)
- **[mobench-sdk](https://crates.io/crates/mobench-sdk)** - Core SDK with timing harness, build automation, and codegen
- **[mobench-macros](https://crates.io/crates/mobench-macros)** - `#[benchmark]` proc macro

## See Also

- [mobench-sdk Documentation](https://crates.io/crates/mobench-sdk) for programmatic API
- [BrowserStack App Automate](https://www.browserstack.com/app-automate) for device cloud
- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/) for FFI details

## License

Licensed under the MIT License. See [LICENSE.md](../../LICENSE.md) for details.

Copyright (c) 2026 World Foundation
